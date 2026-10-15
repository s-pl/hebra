//! Live updates over Server-Sent Events.

use std::time::Duration;

use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};

use crate::api::St;
use crate::error::ApiResult;
use crate::identity::Action;

/// Bookmark moves, presence, reviews, tasks and checks of one repository,
/// as they happen on any replica. Lagging clients get a `resync` event; on
/// shutdown the stream ends and clients reconnect elsewhere.
pub(crate) async fn stream(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
) -> ApiResult<Response> {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use futures_util::StreamExt;
    use tokio_stream::wrappers::BroadcastStream;

    let (repo, _) = s.access(&headers, &owner, &name, Action::Read).await?;
    let repo_id = repo.id.clone();
    let hello = tokio_stream::once(Ok(Event::default()
        .event("ready")
        .retry(Duration::from_secs(2))
        .data("{}")));
    let updates =
        tokio_stream::StreamExt::filter_map(BroadcastStream::new(s.bus.subscribe()), move |item| {
            match item {
                Ok(ev) if ev.repo == repo_id => Some(Ok::<_, axum::Error>(
                    Event::default().event(&ev.kind).data(&ev.json),
                )),
                Ok(_) => None,
                Err(_) => Some(Ok(Event::default().event("resync").data("{}"))),
            }
        });
    let mut shutdown = s.shutdown.subscribe();
    let stop = async move {
        let _ = shutdown.wait_for(|stop| *stop).await;
    };
    let events = hello.chain(updates).take_until(stop);
    let mut response = Sse::new(events.boxed())
        .keep_alive(KeepAlive::default())
        .into_response();
    // Tell proxies not to buffer the stream.
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    Ok(response)
}
