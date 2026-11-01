import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';

export interface Profile {
  displayName: string;
  downloadDir: string;
}
/**
 * `delivery` describes how far an outgoing message has actually got:
 * `pending` (only on this device), `stored` (an encrypted copy was accepted by
 * the mailbox server), `delivered` (the peer acknowledged receipt) or
 * `unknown` (a legacy row with no acknowledgement — never promoted).
 * Incoming messages are always `delivered`.
 */
export type Delivery = 'pending' | 'stored' | 'delivered' | 'unknown';
export interface Message {
  id: string;
  text: string;
  outgoing: boolean;
  timestamp: number;
  delivery: Delivery;
  /** `direct` or `mailbox`; absent while a message is still purely local. */
  via?: string | null;
}
export interface Transfer {
  id: string;
  name: string;
  size: number;
  transferred: number;
  outgoing: boolean;
  status: TransferStatus;
  error?: string | null;
}
/**
 * Outgoing upload: `preparing` → `uploading` → `waiting` (fully staged on the
 * server, waiting for the peer to accept) → `completed`.
 * Incoming download: `offered` → `accepted` → `downloading` → `publishing`
 * (atomic write into the download dir) → `completed`.
 * Any of these can end in `rejected`, `cancelled` or `failed`.
 */
export type TransferStatus =
  | 'preparing'
  | 'uploading'
  | 'waiting'
  | 'offered'
  | 'accepted'
  | 'downloading'
  | 'publishing'
  | 'completed'
  | 'rejected'
  | 'cancelled'
  | 'failed';
export interface ReconnectStatus {
  attempt: number;
  nextRetryMs: number;
}
/** Non-secret room identity; the seed-bearing pairing key is never included. */
export interface RoomInfo {
  role: 'host' | 'join';
  roomId: string;
  networkName: string;
  virtualIp: string;
  peerVirtualIp: string;
}
/** Non-secret server description shown in Settings. */
export interface ServerSummary {
  relayUrl: string;
  mailboxUrl: string;
  mailboxHost: string;
}
/**
 * `state` is offline-mailbox reachability, deliberately separate from whether
 * the peer is online: `notConfigured`, `connecting`, `online`, `retrying`,
 * `error` or `paused`.
 */
export interface MailboxInfo {
  state: string;
  message?: string | null;
  pending: number;
}
export type Connection = 'disconnected' | 'connecting' | 'connected';

export interface Snapshot {
  profile: Profile;
  connection: Connection;
  peerName: string | null;
  messages: Message[];
  transfers: Transfer[];
  engineAvailable: boolean;
  preview: boolean;
  /** A manual disconnect: automatic reconnect stays off until resumed. */
  paused: boolean;
  reconnect: ReconnectStatus | null;
  server: ServerSummary | null;
  room: RoomInfo | null;
  /** "unknown" until real EasyTier RPC/CLI route data exists. */
  route: string;
  /** Offline mailbox reachability; separate from peer reachability. */
  mailbox: MailboxInfo;
}

/** One page of stored history, oldest-first, fixed page size in the core. */
export interface HistoryPage {
  roomId: string;
  messages: Message[];
  hasMore: boolean;
}

export type AppEvent =
  | {
      type: 'state';
      connection: Connection;
      peerName: string | null;
      paused: boolean;
      reconnect: ReconnectStatus | null;
      room: RoomInfo | null;
    }
  | { type: 'server'; server: ServerSummary | null }
  | { type: 'message'; message: Message }
  | { type: 'transfer'; transfer: Transfer }
  | { type: 'mailbox'; state: string; message: string | null; pending: number }
  | { type: 'error'; message: string };

const previewSnapshot: Snapshot = {
  profile: { displayName: '我的电脑', downloadDir: '' },
  connection: 'disconnected',
  peerName: null,
  messages: [],
  transfers: [],
  engineAvailable: false,
  preview: true,
  paused: false,
  reconnect: null,
  server: null,
  room: null,
  route: 'unknown',
  mailbox: { state: 'notConfigured', message: null, pending: 0 },
};

function native<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) return Promise.reject(new Error('当前是界面预览，请启动桌面应用使用此功能。'));
  return invoke<T>(command, args);
}

export interface UpdateInfo { currentVersion: string; version: string | null; notes: string | null }
export interface UpdateProgress { phase: 'downloading' | 'installing'; downloaded: number; total: number | null }

export const api = {
  isDesktop: isTauri,
  checkUpdate: () => native<UpdateInfo>('check_update'),
  installUpdate: () => native<void>('install_update'),
  onUpdateProgress: async (handler: (progress: UpdateProgress) => void): Promise<() => void> => {
    if (!isTauri()) return () => {};
    return listen<UpdateProgress>('update-progress', event => handler(event.payload));
  },
  snapshot: (): Promise<Snapshot> =>
    isTauri() ? invoke('snapshot') : Promise.resolve(structuredClone(previewSnapshot)),

  /** Import a server setup code. Treat it as a credential; never log or display it. */
  importServer: (code: string) => native<void>('import_server', { code }),
  /** Create a hosted room; resolves to the pairing key to share with the peer. */
  createRoom: () => native<string>('create_room'),
  /** Join from a pasted pairing key. */
  joinRoom: (code: string) => native<void>('join_room', { code }),
  /** Explicit export of the current room's pairing key. */
  pairingKey: () => native<string>('pairing_key'),

  /** Manual disconnect; suppresses automatic reconnect until resume. */
  pause: () => native<void>('pause_peer'),
  resume: () => native<void>('resume_peer'),
  /** Explicitly forget the saved room. Chat history is not deleted. */
  forgetRoom: () => native<void>('forget_room'),

  sendMessage: (text: string) => native<Message>('send_message', { text }),
  offerFile: (path: string) => native<Transfer>('offer_file', { path }),
  acceptFile: (id: string) => native<void>('accept_file', { id }),
  rejectFile: (id: string) => native<void>('reject_file', { id }),
  cancelTransfer: (id: string) => native<void>('cancel_transfer', { id }),
  saveProfile: (profile: Profile) => native<void>('save_profile', { profile }),

  /** One page of older messages, strictly before `beforeId` (null = newest). */
  history: (beforeId: string | null): Promise<HistoryPage> =>
    native<HistoryPage>('message_history', { beforeId }),

  chooseFile: async (): Promise<string | null> => {
    if (!isTauri()) throw new Error('请在桌面应用中选择文件。');
    const selected = await open({ multiple: false, directory: false });
    return typeof selected === 'string' ? selected : null;
  },
  chooseFolder: async (): Promise<string | null> => {
    if (!isTauri()) throw new Error('请在桌面应用中选择目录。');
    const selected = await open({ multiple: false, directory: true });
    return typeof selected === 'string' ? selected : null;
  },
  subscribe: async (handler: (event: AppEvent) => void): Promise<() => void> => {
    if (!isTauri()) return () => {};
    return listen<AppEvent>('app-event', event => handler(event.payload));
  },
  onResync: async (handler: (snapshot: Snapshot) => void): Promise<() => void> => {
    if (!isTauri()) return () => {};
    return listen<Snapshot>('app-resync', event => handler(event.payload));
  },
};
