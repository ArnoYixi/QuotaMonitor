/// Appearance derived only from unpainted pixels around an account widget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Appearance {
    pub dark_text: bool,
    pub opacity: u8,
}

impl Appearance {
    pub fn fallback(light_theme: bool) -> Self {
        Self {
            dark_text: light_theme,
            opacity: 0,
        }
    }

    pub fn from_samples(samples: &[[u8; 3]], previous: Self) -> Self {
        // A clipped or unavailable desktop must not turn into a false reading.
        if samples.len() < 12 {
            return previous;
        }
        let n = samples.len() as f32;
        let mut mean = [0.0_f32; 3];
        for rgb in samples {
            for channel in 0..3 {
                mean[channel] += f32::from(rgb[channel]) / n;
            }
        }
        let mut variance = 0.0;
        for rgb in samples {
            for channel in 0..3 {
                variance += (f32::from(rgb[channel]) - mean[channel]).powi(2) / (3.0 * n);
            }
        }
        // RGB variation also catches colorful texture with similar luminance.
        // Ignore compression/antialiasing noise; cap the tint at 58% opacity.
        let detail = ((variance.sqrt() - 14.0) / 66.0).clamp(0.0, 1.0);
        let target = if detail <= 0.0 {
            0
        } else {
            (48.0 + detail * 100.0).round() as u8
        };
        let opacity = if target == 0 || target == 148 || target.abs_diff(previous.opacity) > 5 {
            target
        } else {
            previous.opacity
        };
        let luminance = mean[0] * 0.2126 + mean[1] * 0.7152 + mean[2] * 0.0722;
        // A dead band keeps text/tint polarity stable near middle gray.
        let dark_text = if previous.dark_text {
            luminance >= 135.0
        } else {
            luminance > 165.0
        };
        Self { dark_text, opacity }
    }

    pub fn tint(self) -> [u8; 3] {
        if self.dark_text {
            [248, 250, 252]
        } else {
            [12, 16, 20]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_backgrounds_remain_transparent_and_choose_readable_text() {
        let dark = Appearance::from_samples(&[[10, 18, 24]; 64], Appearance::fallback(true));
        let light = Appearance::from_samples(&[[246, 248, 250]; 64], dark);
        assert_eq!(dark.opacity, 0);
        assert!(!dark.dark_text);
        assert_eq!(light.opacity, 0);
        assert!(light.dark_text);
    }

    #[test]
    fn desktop_icon_blocks_add_a_bounded_backdrop_and_recover() {
        let pixels: Vec<_> = (0..64)
            .map(|i| if i % 3 == 0 { [250; 3] } else { [8, 16, 30] })
            .collect();
        let busy = Appearance::from_samples(&pixels, Appearance::fallback(false));
        assert!(busy.opacity >= 130 && busy.opacity <= 148);
        let stable = Appearance::from_samples(&pixels, busy);
        assert_eq!(stable, busy);
        assert_eq!(
            Appearance::from_samples(&[[8, 16, 30]; 64], busy).opacity,
            0
        );
    }

    #[test]
    fn similar_luminance_colors_still_need_a_backdrop() {
        let pixels: Vec<_> = (0..64)
            .map(|i| {
                if i % 2 == 0 {
                    [220, 40, 40]
                } else {
                    [0, 100, 0]
                }
            })
            .collect();
        assert!(Appearance::from_samples(&pixels, Appearance::fallback(false)).opacity > 100);
    }

    #[test]
    fn low_noise_and_small_luminance_changes_do_not_flicker() {
        let noise: Vec<_> = (0..64).map(|i| [30 + (i % 8) as u8; 3]).collect();
        let mut previous = Appearance::from_samples(&noise, Appearance::fallback(false));
        assert_eq!(previous.opacity, 0);
        for gray in [149, 155, 151, 160] {
            previous = Appearance::from_samples(&[[gray; 3]; 64], previous);
            assert!(!previous.dark_text);
        }
        assert_eq!(Appearance::from_samples(&[], previous), previous);
    }
}
