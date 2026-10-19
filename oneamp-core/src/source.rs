//! Turn a [`MediaDescriptor`] into a running decoder. Runs on a decode
//! worker: every step may block (disk, network, host callbacks).

use crate::error::PlayerError;
use crate::media::{CancelToken, MediaDescriptor, MediaLocation};
use crate::symphonia_player::SymphoniaPlayer;
use crate::track::TrackInfo;
use std::sync::Arc;
use std::sync::atomic::AtomicU32;

#[cfg(feature = "http")]
use crate::http_stream::{IcySnapshot, ReconnectSnapshot};

pub(crate) struct OpenedSource {
    pub player: SymphoniaPlayer,
    pub track: TrackInfo,
    pub live: bool,
    #[cfg(feature = "http")]
    pub icy: Option<IcySnapshot>,
    #[cfg(feature = "http")]
    pub reconnect: Option<ReconnectSnapshot>,
    /// Transport reconnects (on-demand HTTP), for diagnostics.
    pub reconnects: Option<Arc<AtomicU32>>,
}

/// Transport settings the engine passes down.
#[derive(Debug, Clone, Default)]
pub(crate) struct SourceConfig {
    #[cfg(feature = "http")]
    pub http: crate::http_client::HttpConfig,
}

pub(crate) fn open(
    desc: &MediaDescriptor,
    cfg: &SourceConfig,
    cancel: &CancelToken,
) -> Result<OpenedSource, PlayerError> {
    cancel.check()?;
    let _ = cfg;
    match &desc.location {
        MediaLocation::File(path) => {
            let mut player = SymphoniaPlayer::load(path)?;
            let size = std::fs::metadata(path).ok().map(|m| m.len());
            let track = TrackInfo::assemble(desc, &mut player, size);
            Ok(plain(player, track, false))
        }
        MediaLocation::Custom(factory) => {
            let source = factory.open(cancel)?;
            let len = source.byte_len();
            let mut player = SymphoniaPlayer::open(source, &desc.format)?;
            let track = TrackInfo::assemble(desc, &mut player, len);
            Ok(plain(player, track, false))
        }
        #[cfg(feature = "http")]
        MediaLocation::Http { access, delivery } => {
            open_http(desc, access.clone(), *delivery, cfg, cancel)
        }
    }
}

fn plain(player: SymphoniaPlayer, track: TrackInfo, live: bool) -> OpenedSource {
    OpenedSource {
        player,
        track,
        live,
        #[cfg(feature = "http")]
        icy: None,
        #[cfg(feature = "http")]
        reconnect: None,
        reconnects: None,
    }
}

#[cfg(feature = "http")]
fn open_http(
    desc: &MediaDescriptor,
    provider: Arc<dyn crate::access::AccessProvider>,
    delivery: crate::media::Delivery,
    cfg: &SourceConfig,
    cancel: &CancelToken,
) -> Result<OpenedSource, PlayerError> {
    use crate::http_range::RangeSource;
    use crate::http_stream::{HttpStream, live_headers};
    use crate::media::{Delivery, FormatHint, is_segmented_mime};

    let segmented = || {
        PlayerError::unsupported(
            "segmented delivery (HLS/DASH) needs an adapter: supply it as MediaLocation::Custom",
        )
    };
    if desc
        .format
        .mime_type
        .as_deref()
        .is_some_and(is_segmented_mime)
        || desc
            .format
            .extension
            .as_deref()
            .is_some_and(|e| e.eq_ignore_ascii_case("m3u8") || e.eq_ignore_ascii_case("mpd"))
    {
        return Err(segmented());
    }

    let http = &cfg.http;
    let agent = crate::http_client::agent(http);
    let mut access = provider.access()?;
    let extra: Vec<(&str, String)> = if delivery == Delivery::OnDemand {
        vec![("Accept", "*/*".into())]
    } else {
        live_headers()
    };
    let probe = RangeSource::probe(&agent, provider.as_ref(), &mut access, &extra, http, cancel)?;
    let content_type = probe.response.header("content-type").map(str::to_string);
    if content_type.as_deref().is_some_and(is_segmented_mime) {
        return Err(segmented());
    }
    let icy = probe.response.header("icy-metaint").is_some()
        || probe.response.header("icy-name").is_some();
    let live = match delivery {
        Delivery::Live => true,
        Delivery::OnDemand => false,
        // Range support proves a finite file; ICY headers or a body with
        // no announced length mean a live stream.
        Delivery::Auto => icy || (!probe.ranged && probe.len.is_none()),
    };
    let hint = FormatHint {
        extension: desc.format.extension.clone(),
        mime_type: desc.format.mime_type.clone().or(content_type),
    };

    if live {
        let stream = HttpStream::from_response(
            probe.response,
            agent,
            *http,
            provider,
            access,
            cancel.clone(),
        );
        let icy = stream.icy_title_handle();
        let reconnect = stream.reconnect_state_handle();
        let mut player = SymphoniaPlayer::open(Box::new(stream), &hint)?;
        let mut track = TrackInfo::assemble(desc, &mut player, None);
        track.live = true;
        track.duration = None;
        if track.codec.is_some() {
            track.codec = track.codec.map(|c| format!("{c} STREAM"));
        }
        return Ok(OpenedSource {
            player,
            track,
            live: true,
            icy: Some(icy),
            reconnect: Some(reconnect),
            reconnects: None,
        });
    }

    let len = probe.len;
    let source = RangeSource::from_probe(agent, *http, provider, access, probe, cancel.clone());
    let reconnects = source.reconnects.clone();
    let transport = source.last_error.clone();
    let mut player = SymphoniaPlayer::open(Box::new(source), &hint).map_err(|e| {
        // A transport failure during probing is the real cause.
        transport
            .lock()
            .ok()
            .and_then(|mut t| t.take())
            .unwrap_or(e)
    })?;
    let track = TrackInfo::assemble(desc, &mut player, len);
    Ok(OpenedSource {
        player,
        track,
        live: false,
        icy: None,
        reconnect: None,
        reconnects: Some(reconnects),
    })
}
