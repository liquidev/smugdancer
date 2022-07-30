//! Color space conversions.

fn gamma(x: f32) -> f32 {
    if x >= 0.0031308 {
        (1.055) * x.powf(1.0 / 2.4) - 0.055
    } else {
        12.92 * x
    }
}

fn gamma_inv(x: f32) -> f32 {
    if x >= 0.04045 {
        ((x + 0.055) / (1.0 + 0.055)).powf(2.4)
    } else {
        x / 12.92
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Srgb {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Srgb {
    pub fn from_array(array: [u8; 3]) -> Self {
        Self {
            r: array[0] as f32 / 255.0,
            g: array[1] as f32 / 255.0,
            b: array[2] as f32 / 255.0,
        }
    }

    pub fn to_array(self) -> [u8; 3] {
        [
            (self.r * 255.0) as u8,
            (self.g * 255.0) as u8,
            (self.b * 255.0) as u8,
        ]
    }

    pub fn to_linear(self) -> LinearRgb {
        LinearRgb {
            r: gamma_inv(self.r),
            g: gamma_inv(self.g),
            b: gamma_inv(self.b),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LinearRgb {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl LinearRgb {
    pub fn to_srgb(self) -> Srgb {
        Srgb {
            r: gamma(self.r),
            g: gamma(self.g),
            b: gamma(self.b),
        }
    }

    #[allow(clippy::excessive_precision)]
    pub fn to_oklab(self) -> Oklab {
        let l = 0.4122214708 * self.r + 0.5363325363 * self.g + 0.0514459929 * self.b;
        let m = 0.2119034982 * self.r + 0.6806995451 * self.g + 0.1073969566 * self.b;
        let s = 0.0883024619 * self.r + 0.2817188376 * self.g + 0.6299787005 * self.b;

        let l_ = l.cbrt();
        let m_ = m.cbrt();
        let s_ = s.cbrt();

        Oklab {
            l: 0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_,
            a: 1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_,
            b: 0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Oklab {
    pub l: f32,
    pub a: f32,
    pub b: f32,
}

impl Oklab {
    #[allow(clippy::excessive_precision)]
    pub fn to_linear(self) -> LinearRgb {
        let l_ = self.l + 0.3963377774 * self.a + 0.2158037573 * self.b;
        let m_ = self.l - 0.1055613458 * self.a - 0.0638541728 * self.b;
        let s_ = self.l - 0.0894841775 * self.a - 1.2914855480 * self.b;

        let l = l_ * l_ * l_;
        let m = m_ * m_ * m_;
        let s = s_ * s_ * s_;

        LinearRgb {
            r: 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
            g: -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
            b: -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
        }
    }
}
