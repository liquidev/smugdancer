use nanorand::{Rng, WyRand};

use crate::{colorspace::Oklab, image::Image};

#[derive(Debug)]
struct Mean {
    position: [f32; 3],
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

pub fn extract_palette(image: &Image<Oklab>, colors: usize, iterations: usize) -> Vec<Oklab> {
    let observations = {
        let mut observations: Vec<_> = image
            .pixels
            .iter()
            .map(|color| [color.l, color.a, color.b])
            .collect();
        observations.sort_by(|a, b| a.partial_cmp(b).unwrap());
        observations.dedup();
        observations
    };

    let mut rng = WyRand::new_seed(2137);
    let mut means: Vec<_> = (0..colors)
        .map(|_| Mean {
            // TODO: k-means++
            position: observations[rng.generate_range(0..observations.len())],
            observations: vec![],
        })
        .collect();

    for _ in 0..iterations {
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
    }

    means
        .iter()
        .map(
            |&Mean {
                 position: [l, a, b],
                 ..
             }| Oklab { l, a, b },
        )
        .collect()
}
