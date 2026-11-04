//! Transport adapter for core-authorized local image reads. Never a path API.
use crate::AppState;
use tauri::{
    http::{Request, Response, StatusCode},
    Manager,
};

fn response(status: StatusCode, mime: &str, body: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("Content-Type", mime)
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .header("Content-Security-Policy", "default-src 'none'; sandbox")
        .body(body)
        .unwrap()
}

fn trusted_origin(origin: &str) -> bool {
    matches!(
        origin,
        "tauri://localhost" | "http://tauri.localhost" | "https://tauri.localhost"
    ) || (cfg!(debug_assertions)
        && matches!(origin, "http://localhost:1420" | "http://127.0.0.1:1420"))
}

fn identifiers(request: &Request<Vec<u8>>) -> Option<(&str, &str)> {
    if request.method() != "GET" || request.uri().query().is_some() {
        return None;
    }
    if let Some(origin) = request.headers().get("Origin") {
        if !trusted_origin(origin.to_str().ok()?) {
            return None;
        }
    }
    let (room, id) = request.uri().path().strip_prefix('/')?.split_once('_')?;
    if room.len() != 64
        || !room
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return None;
    }
    if uuid::Uuid::parse_str(id).ok()?.to_string() != id {
        return None;
    }
    Some((room, id))
}

pub fn register(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder.register_asynchronous_uri_scheme_protocol(
        "twinlink-preview",
        |ctx, request, responder| {
            let denied = || response(StatusCode::NOT_FOUND, "text/plain", Vec::new());
            if ctx.webview_label() != "main" {
                responder.respond(denied());
                return;
            }
            let app = ctx.app_handle();
            let trusted = app
                .get_webview_window("main")
                .and_then(|window| window.url().ok())
                .is_some_and(|url| {
                    let port = url
                        .port()
                        .map(|port| format!(":{port}"))
                        .unwrap_or_default();
                    trusted_origin(&format!(
                        "{}://{}{port}",
                        url.scheme(),
                        url.host_str().unwrap_or_default()
                    ))
                });
            let Some((room, id)) = identifiers(&request).filter(|_| trusted) else {
                responder.respond(denied());
                return;
            };
            let Some(state) = app.try_state::<AppState>() else {
                responder.respond(denied());
                return;
            };
            let core = state.0.clone();
            let (room, id) = (room.to_owned(), id.to_owned());
            tauri::async_runtime::spawn(async move {
                let result = twinlink_core::preview::preview_image(&core, &room, &id).await;
                responder.respond(match result {
                    Ok(image) => response(StatusCode::OK, image.mime, image.bytes),
                    Err(_) => response(StatusCode::NOT_FOUND, "text/plain", Vec::new()),
                });
            });
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_canonical_ids_and_trusted_requests() {
        let path = format!(
            "twinlink-preview://localhost/{}_{}",
            "a".repeat(64),
            uuid::Uuid::new_v4()
        );
        assert!(identifiers(&Request::builder().uri(&path).body(vec![]).unwrap()).is_some());
        for uri in [
            format!("{path}?path=/etc/passwd"),
            "twinlink-preview://localhost/etc/passwd".into(),
        ] {
            assert!(identifiers(&Request::builder().uri(uri).body(vec![]).unwrap()).is_none());
        }
        assert!(identifiers(
            &Request::builder()
                .uri(&path)
                .header("Origin", "https://evil.invalid")
                .body(vec![])
                .unwrap()
        )
        .is_none());
        assert!(identifiers(
            &Request::builder()
                .method("POST")
                .uri(path)
                .body(vec![])
                .unwrap()
        )
        .is_none());
        let result = response(StatusCode::OK, "image/png", vec![]);
        assert_eq!(result.headers()["Cache-Control"], "no-store");
        assert!(result
            .headers()
            .get("Access-Control-Allow-Origin")
            .is_none());
    }
}
