#!/usr/bin/env bash
# Run as root on the selected Debian/Linux host, with server/mailbox.py beside
# this script as ./mailbox.py. No SSH password, room key, or operator token is
# accepted on the command line. Existing SSH/443 services are left untouched.
set -euo pipefail
umask 077
[[ $(id -u) == 0 ]] || { echo 'Run this deployment script as root.' >&2; exit 1; }
[[ $(uname -m) == x86_64 ]] || { echo 'This pinned deployment currently supports Linux x86_64.' >&2; exit 1; }
for command in python3 openssl systemctl useradd install ss ufw; do
  command -v "$command" >/dev/null || { echo "Missing prerequisite: $command" >&2; exit 1; }
done
[[ -f ./mailbox.py ]] || { echo 'Place the reviewed server/mailbox.py in this directory first.' >&2; exit 1; }
for port in 11010 18443; do
  if ss -lntup | awk '{print $5}' | grep -Eq ":${port}$"; then
    if ! systemctl is-active --quiet twinlink-net && ! systemctl is-active --quiet twinlink-mailbox; then
      echo "Port $port is already occupied; refusing to replace an existing service." >&2
      exit 1
    fi
  fi
done
install -d -m 755 /opt/twinlink
install -d -m 700 /etc/twinlink
for service_user in twinlink-net twinlink-mailbox; do
  if ! id "$service_user" >/dev/null 2>&1; then
    useradd --system --user-group --home-dir "/var/lib/$service_user" --no-create-home --shell /usr/sbin/nologin "$service_user"
  fi
done

python3 - <<'PY'
import hashlib, io, json, os, pathlib, secrets, urllib.request, zipfile
root = pathlib.Path('/etc/twinlink')
settings = root / 'server-settings.json'
if not settings.exists():
    settings.write_text(json.dumps({'networkPrefix': 'tl-' + secrets.token_hex(16),
                                   'accessToken': secrets.token_urlsafe(32)}, indent=2))
    settings.chmod(0o600)
config = json.loads(settings.read_text())
assert len(config['accessToken']) >= 32
assert config['networkPrefix'].startswith('tl-') and len(config['networkPrefix']) == 35
env = root / 'mailbox.env'
env.write_text('TWINLINK_OPERATOR_TOKEN=' + config['accessToken'] + '\n')
env.chmod(0o600)
network = root / 'network.env'
if not network.exists():
    network.write_text('ET_NETWORK_NAME=tl-relay-' + secrets.token_hex(16) + '\nET_NETWORK_SECRET=' + secrets.token_hex(32) + '\n')
with network.open('a') as f:
    # Repeating this harmless assignment keeps updates idempotent.
    f.write('TWINLINK_RELAY_WHITELIST=' + config['networkPrefix'] + '-*\n')
network.chmod(0o600)
version = 'v2.6.4'
name = 'easytier-linux-x86_64-' + version + '.zip'
def request(url):
    return urllib.request.urlopen(urllib.request.Request(url, headers={'User-Agent': 'TwinLink-private-deploy/1'}), timeout=120)
with request('https://api.github.com/repos/EasyTier/EasyTier/releases/tags/' + version) as r:
    release = json.load(r)
asset = next(a for a in release['assets'] if a['name'] == name)
with request(asset['browser_download_url']) as r:
    archive = r.read(150 * 1024 * 1024)
digest = hashlib.sha256(archive).hexdigest()
assert asset.get('digest') == 'sha256:' + digest, 'Official release SHA-256 missing or mismatched'
with zipfile.ZipFile(io.BytesIO(archive)) as z:
    for filename in ['easytier-core', 'easytier-cli']:
        member = next(i for i in z.infolist() if pathlib.PurePosixPath(i.filename).name == filename)
        output = pathlib.Path('/opt/twinlink') / filename
        output.write_bytes(z.read(member))
        output.chmod(0o755)
pathlib.Path('/opt/twinlink/engine-version.json').write_text(json.dumps({'version': version, 'asset': name, 'sha256': digest}, indent=2))
print('Official EasyTier archive SHA-256 verified:', digest)
PY

install -m 755 ./mailbox.py /opt/twinlink/mailbox.py
install -d -m 700 -o twinlink-mailbox -g twinlink-mailbox /etc/twinlink/tls
if [[ ! -f /etc/twinlink/ca-cert.pem ]]; then
  if [[ -f /etc/twinlink/tls/key.pem && -f /etc/twinlink/tls/cert.pem ]]; then
    # Migrate the original self-signed CA without invalidating setup codes.
    openssl x509 -in /etc/twinlink/tls/cert.pem -out /etc/twinlink/ca-cert.pem
    install -m 600 /etc/twinlink/tls/key.pem /etc/twinlink/ca-key.pem
  else
    openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 3650 \
      -keyout /etc/twinlink/ca-key.pem -out /etc/twinlink/ca-cert.pem \
      -subj '/CN=TwinLink Private CA' -addext 'basicConstraints=critical,CA:TRUE' \
      -addext 'keyUsage=critical,keyCertSign,cRLSign'
  fi
fi
chown root:root /etc/twinlink/ca-key.pem /etc/twinlink/ca-cert.pem
chmod 600 /etc/twinlink/ca-key.pem
chmod 644 /etc/twinlink/ca-cert.pem
if [[ ! -f /etc/twinlink/tls/cert.pem ]] \
  || openssl x509 -in /etc/twinlink/tls/cert.pem -noout -text | grep -q 'CA:TRUE' \
  || ! openssl x509 -in /etc/twinlink/tls/cert.pem -checkend 2592000 -noout >/dev/null; then
  openssl x509 -in /etc/twinlink/ca-cert.pem -checkend 2678400 -noout >/dev/null \
    || { echo 'CA expires within 31 days; rotate CA and re-provision clients before renewing.' >&2; exit 1; }
  ca_end=$(openssl x509 -in /etc/twinlink/ca-cert.pem -noout -enddate)
  leaf_days=$(python3 -c 'import ssl,sys,time; print(min(365, int((ssl.cert_time_to_seconds(sys.argv[1])-time.time())//86400)-1))' "${ca_end#notAfter=}")
  tls_stage=$(mktemp -d /etc/twinlink/.tls-stage.XXXXXX)
  trap 'rm -rf "$tls_stage"' EXIT
  openssl req -new -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
    -keyout "$tls_stage/key.pem" -out "$tls_stage/request.pem" -subj '/CN=TwinLink Private Mailbox'
  cat > "$tls_stage/extensions.cnf" <<'TLS'
basicConstraints=critical,CA:FALSE
keyUsage=critical,digitalSignature
extendedKeyUsage=serverAuth
subjectAltName=IP:45.76.70.254
TLS
  openssl x509 -req -in "$tls_stage/request.pem" -CA /etc/twinlink/ca-cert.pem \
    -CAkey /etc/twinlink/ca-key.pem -set_serial "0x$(openssl rand -hex 16)" \
    -days "$leaf_days" -sha256 -extfile "$tls_stage/extensions.cnf" -out "$tls_stage/leaf.pem"
  openssl verify -CAfile /etc/twinlink/ca-cert.pem "$tls_stage/leaf.pem"
  cat "$tls_stage/leaf.pem" /etc/twinlink/ca-cert.pem > "$tls_stage/chain.pem"
  install -m 600 -o twinlink-mailbox -g twinlink-mailbox "$tls_stage/key.pem" /etc/twinlink/tls/key.pem
  install -m 644 -o twinlink-mailbox -g twinlink-mailbox "$tls_stage/chain.pem" /etc/twinlink/tls/cert.pem
  rm -rf "$tls_stage"
  trap - EXIT
fi
chown twinlink-mailbox:twinlink-mailbox /etc/twinlink/tls/key.pem /etc/twinlink/tls/cert.pem
chmod 600 /etc/twinlink/tls/key.pem
chmod 644 /etc/twinlink/tls/cert.pem
# The parent grants traversal only to the dedicated mailbox group; settings and
# environment files remain readable only by root/systemd.
chown root:twinlink-mailbox /etc/twinlink
chmod 710 /etc/twinlink

cat > /etc/systemd/system/twinlink-net.service <<'UNIT'
[Unit]
Description=TwinLink private EasyTier discovery and relay
After=network-online.target
Wants=network-online.target

[Service]
User=twinlink-net
Group=twinlink-net
EnvironmentFile=/etc/twinlink/network.env
WorkingDirectory=/var/lib/twinlink-net
StateDirectory=twinlink-net
StateDirectoryMode=0700
ExecStart=/opt/twinlink/easytier-core --hostname twinlink-relay --listeners tcp://0.0.0.0:11010 udp://0.0.0.0:11010 --no-tun true --use-smoltcp true --rpc-portal 127.0.0.1:15888 --relay-network-whitelist ${TWINLINK_RELAY_WHITELIST} --console-log-level warn --file-log-level off
Restart=on-failure
RestartSec=5
NoNewPrivileges=true
PrivateTmp=true
ProtectHome=true
ProtectSystem=strict
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
MemoryMax=256M
TasksMax=128
LimitNOFILE=8192

[Install]
WantedBy=multi-user.target
UNIT

cat > /etc/systemd/system/twinlink-mailbox.service <<'UNIT'
[Unit]
Description=TwinLink private encrypted offline mailbox
After=network-online.target
Wants=network-online.target

[Service]
User=twinlink-mailbox
Group=twinlink-mailbox
EnvironmentFile=/etc/twinlink/mailbox.env
WorkingDirectory=/var/lib/twinlink-mailbox
StateDirectory=twinlink-mailbox
StateDirectoryMode=0700
ExecStart=/usr/bin/python3 -I /opt/twinlink/mailbox.py --host 0.0.0.0 --port 18443 --data /var/lib/twinlink-mailbox --cert /etc/twinlink/tls/cert.pem --key /etc/twinlink/tls/key.pem --max-storage-mib 4096 --max-file-mib 1024 --ttl-hours 168
Restart=on-failure
RestartSec=5
NoNewPrivileges=true
PrivateTmp=true
ProtectHome=true
ProtectSystem=strict
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
UMask=0077
MemoryMax=192M
TasksMax=64
LimitNOFILE=4096

[Install]
WantedBy=multi-user.target
UNIT

systemd-analyze verify /etc/systemd/system/twinlink-net.service /etc/systemd/system/twinlink-mailbox.service
systemctl daemon-reload
systemctl enable twinlink-net twinlink-mailbox
systemctl restart twinlink-net twinlink-mailbox
sleep 2
systemctl is-active twinlink-net twinlink-mailbox
python3 - <<'PY'
import json, ssl, urllib.request
context = ssl.create_default_context(cafile='/etc/twinlink/ca-cert.pem')
# Check the certificate's actual public-IP identity, using a loopback TCP
# connection would require a separate localhost SAN. The external listener is
# checked after the narrowly scoped firewall rules are added below.
import socket
with socket.create_connection(('127.0.0.1', 18443), timeout=10) as s:
    with context.wrap_socket(s, server_hostname='45.76.70.254') as tls:
        tls.sendall(b'GET /health HTTP/1.1\r\nHost: 45.76.70.254\r\nConnection: close\r\n\r\n')
        result = tls.recv(4096)
        assert b'200 OK' in result, result[:100]
print('TLS health check passed with certificate and hostname verification')
PY
ufw allow 11010/tcp comment 'TwinLink relay TCP'
ufw allow 11010/udp comment 'TwinLink relay UDP'
ufw allow 18443/tcp comment 'TwinLink private mailbox TLS'
ss -lntup | grep -E ':11010 |:18443 |:15888 ' || true
openssl x509 -in /etc/twinlink/tls/cert.pem -noout -fingerprint -sha256 -enddate
echo 'TwinLink services deployed. Credentials remain in /etc/twinlink, not in this output.'
