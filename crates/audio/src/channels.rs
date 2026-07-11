//! Channel mapping to interleaved stereo.

/// Append `src` (interleaved, `channels` channels) to `out` as interleaved stereo. Mono
/// duplicates to both channels, stereo passes through, and more than two channels take the
/// front left/right pair for now (a fuller downmix matrix comes later).
pub fn to_stereo(src: &[f32], channels: usize, out: &mut Vec<f32>) {
    match channels {
        0 => {}
        1 => {
            for &s in src {
                out.push(s);
                out.push(s);
            }
        }
        2 => out.extend_from_slice(src),
        n => {
            for frame in src.chunks_exact(n) {
                out.push(frame[0]);
                out.push(frame[1]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_duplicates_to_both_channels() {
        let mut out = Vec::new();
        to_stereo(&[0.5, -0.5], 1, &mut out);
        assert_eq!(out, vec![0.5, 0.5, -0.5, -0.5]);
    }

    #[test]
    fn stereo_passes_through() {
        let mut out = Vec::new();
        to_stereo(&[0.1, 0.2, 0.3, 0.4], 2, &mut out);
        assert_eq!(out, vec![0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn multichannel_takes_front_lr() {
        let mut out = Vec::new();
        // two frames of 4-channel audio: [FL, FR, RL, RR]
        to_stereo(&[1.0, 2.0, 9.0, 9.0, 3.0, 4.0, 9.0, 9.0], 4, &mut out);
        assert_eq!(out, vec![1.0, 2.0, 3.0, 4.0]);
    }
}
