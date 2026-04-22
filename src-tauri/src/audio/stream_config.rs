use cpal::{SampleFormat, SupportedBufferSize, SupportedStreamConfigRange};

use super::runtime_state::AudioError;

fn pick_sample_rate(cfg: &SupportedStreamConfigRange, requested: u32) -> (u32, u32) {
    let min = cfg.min_sample_rate().0;
    let max = cfg.max_sample_rate().0;
    if requested < min {
        (min, min - requested)
    } else if requested > max {
        (max, requested - max)
    } else {
        (requested, 0)
    }
}

pub(super) fn select_output_config(
    supported: &[SupportedStreamConfigRange],
    requested_rate: u32,
) -> Result<(SupportedStreamConfigRange, u32), AudioError> {
    supported
        .iter()
        .min_by_key(|cfg| {
            let (_, distance) = pick_sample_rate(cfg, requested_rate);
            let format_score = if cfg.sample_format() == SampleFormat::F32 {
                0
            } else {
                1
            };
            let channel_score = if cfg.channels() == 2 { 0 } else { 1 };
            (distance, format_score, channel_score)
        })
        .map(|cfg| {
            let (actual_rate, _) = pick_sample_rate(cfg, requested_rate);
            (*cfg, actual_rate)
        })
        .ok_or_else(|| AudioError::Message("No supported output config found.".into()))
}

pub(super) fn choose_buffer_size(supported: &SupportedBufferSize, requested: u32) -> u32 {
    match supported {
        SupportedBufferSize::Range { min, max } => requested.max(*min).min(*max),
        SupportedBufferSize::Unknown => requested,
    }
}

pub(super) fn max_plugin_block_size(supported: &SupportedBufferSize, requested: u32) -> u32 {
    match supported {
        SupportedBufferSize::Range { min, max } => requested.max(*min).max(*max),
        SupportedBufferSize::Unknown => requested.max(2048),
    }
}

pub(super) fn buffer_fallback_candidates(
    supported: &SupportedBufferSize,
    preferred: u32,
) -> Vec<u32> {
    let common_sizes = [
        64u32, 96, 128, 192, 256, 384, 480, 512, 768, 1024, 1536, 2048,
    ];
    let mut extras: Vec<u32> = common_sizes
        .into_iter()
        .filter(|size| match supported {
            SupportedBufferSize::Range { min, max } => *size >= *min && *size <= *max,
            SupportedBufferSize::Unknown => true,
        })
        .filter(|size| *size != preferred)
        .collect();

    extras.sort_by_key(|size| size.abs_diff(preferred));
    extras.dedup();

    let mut out = Vec::with_capacity(extras.len() + 1);
    out.push(preferred);
    out.extend(extras);
    out
}
