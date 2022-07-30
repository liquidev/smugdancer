use std::fmt::Write;

use image::{Rgb, RgbImage};
use nanorand::{Rng, WyRand};

#[derive(Debug)]
struct Mean {
    position: [f32; 3],
    color: [u8; 3],
    observations: Vec<[f32; 3]>,
}

fn distance_squared(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let dz = b[2] - a[2];
    dx * dx + dy * dy + dz * dz
}

fn find_closest_mean(observation: [f32; 3], means: &[Mean]) -> usize {
    let (mut min_index, mut min_distance) = (0, f32::INFINITY);
    for (i, mean) in means.iter().enumerate() {
        let distance = distance_squared(observation, mean.position);
        if distance < min_distance {
            min_distance = distance;
            min_index = i;
        }
    }
    min_index
}

pub fn extract_palette(image: &RgbImage, colors: usize, iterations: usize) -> Vec<[u8; 3]> {
    let observations = {
        let mut observations: Vec<_> = image
            .pixels()
            .map(|Rgb(color)| {
                [
                    color[0] as f32 / 255.0,
                    color[1] as f32 / 255.0,
                    color[2] as f32 / 255.0,
                ]
            })
            .collect();
        observations.sort_by(|a, b| a.partial_cmp(b).unwrap());
        observations.dedup();
        observations
    };

    let mut observation_circles = String::new();
    for &observation in &observations {
        let rgb = observation.map(|x| (x * 255.0) as u8);
        let _ = write!(
            observation_circles,
            r##"<circle cx="{}" cy="{}" r="0.0025"
                    fill="#{:02x}{:02x}{:02x}"/>"##,
            observation[0], observation[1], rgb[0], rgb[1], rgb[2]
        );
    }

    let mut rng = WyRand::new_seed(2137);
    let mut means: Vec<_> = (0..colors)
        .map(|_| Mean {
            position: observations[rng.generate_range(0..observations.len())],
            color: [rng.generate(), rng.generate(), rng.generate()],
            observations: vec![],
        })
        .collect();
    println!("{:#?}", means);

    for i in 0..iterations {
        let mut means_dbg = String::new();

        println!("iteration #{i}");
        for mean in &mut means {
            mean.observations.clear();
        }

        for &observation in &observations {
            let closest = find_closest_mean(observation, &means);
            means[closest].observations.push(observation);
        }

        for mean in &mut means {
            if let Some(sum) = mean
                .observations
                .iter()
                .copied()
                .reduce(|[a, b, c], [x, y, z]| [a + x, b + y, c + z])
            {
                mean.position = sum.map(|x| x / mean.observations.len() as f32);
            }
        }

        for mean in &means {
            let mut group = String::new();
            for observation in &mean.observations {
                let _ = write!(
                    group,
                    r#"<circle cx="{}" cy="{}" r="0.001"/>"#,
                    observation[0], observation[1]
                );
            }
            let _ = write!(
                means_dbg,
                r##"
                    <g fill="#{:02x}{:02x}{:02x}">
                        <circle cx="{}" cy="{}" r="0.005"/>
                        {group}
                    </g>
                "##,
                mean.color[0], mean.color[1], mean.color[2], mean.position[0], mean.position[1]
            );
        }

        let svg = format!(
            r#"
                <svg viewBox="0 0 512 512" xmlns="http://www.w3.org/2000/svg">
                    <g transform="scale(512 512) scale(1 -1) translate(0 -1)">
                        <g stroke="black" stroke-width="0.0001">{observation_circles}</g>
                        {means_dbg}
                    </g>
                </svg>
            "#
        );
        std::fs::write(format!("/tmp/iteration-{i}.svg"), svg).unwrap();
    }

    means
        .iter()
        .map(|mean| mean.position.map(|x| (x * 255.0) as u8))
        .collect()
}
