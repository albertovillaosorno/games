// Copyright:
//   - Copyright © 2026 Alberto Villa Osorno.
// SPDX-License-Identifier:
//   - Apache-2.0
// Confidential:
//   - false
// License-File:
//   - LICENSE
//
// Boundary-Contract:
// - Owns:
//   - Finite changing-scene reference reconstruction workload composition.
// - Must-Not:
//   - Simulate original gameplay, infer scanout latency or publish pixels.
// - Allows:
//   - Inputs: Verified bundle and explicit aligned scene observation paths.
//   - Outputs: Exact comparison admission and host workload timing summary.
//   - Side effects: Reads private observations and opens a hidden window.
// - Split-When:
//   - A representative interactive title benchmark becomes available.
// - Merge-When:
//   - Another workload shares the same reference scene reconstruction.
// - Summary:
//   - Cycle admitted changing scenes through one persistent native texture.
// - Description:
//   - Verify every scene first, then reconstruct and present timed frames.
// - Usage:
//   - Supply two or more distinct complete reference scenes.
// - Defaults:
//   - Reject differences and excessive workload; no gameplay parity claim.

//! Changing-scene reconstruction and streaming presentation measurement.

use std::ffi::OsString;
use std::num::NonZeroU32;

use games_platform::ir::frame::RenderError;
use games_platform::ir::graphics::{FrameSession, NativeWindow};
use games_platform::ir::importer::reference::{Machine, Pixel};
use games_platform::ir::measurement::now;
use games_platform::ir::storage::bundle::CanonicalBundle;

use super::{
    Arguments, GraphicsPayloads, Observation, SceneAssets, count_matches,
    native_layout,
};
use crate::render_benchmark::{self, Schedule};
use crate::{USAGE, text};

const GROUP: usize = 7;
const MAX_OBSERVATIONS: usize = 64;
const MAX_PRESENTATIONS: u32 = 20_000;

/// Explicit bounded sequence of complete aligned scene observations.
#[derive(Debug)]
pub struct Sequence {
    frames: NonZeroU32,
    paths: Vec<Arguments>,
}

impl Sequence {
    fn load(
        &self,
        machine: Machine,
        assets: &SceneAssets<'_>,
        pens: &mut [Pixel],
    ) -> Result<Vec<Observation>, String> {
        let mut observations = Vec::new();
        observations
            .try_reserve_exact(self.paths.len())
            .map_err(|_error| String::from("observation allocation failed"))?;
        let mut first = None;
        let mut changed = false;
        for (index, path) in self.paths.iter().enumerate() {
            let (observation, reference) = Observation::load(path, machine)?;
            observation.reconstruct(machine, assets, pens)?;
            if count_matches(pens, &observation.colors, &reference)?
                != pens.len()
            {
                return Err(format!("scene {index} differs from reference"));
            }
            if let Some(original) = &first {
                changed |= original != &reference;
            } else {
                first = Some(reference);
            }
            observations.push(observation);
        }
        if !changed {
            return Err(String::from("scene corpus has no visible changes"));
        }
        Ok(observations)
    }

    /// Parse explicit scene groups and repetition count.
    /// # Errors
    /// Rejects missing, partial, too few or excessive scene groups or work.
    pub fn parse(args: &[OsString]) -> Result<Self, String> {
        let (header, groups) = args
            .split_first_chunk::<3>()
            .ok_or_else(|| String::from(USAGE))?;
        let [parent, digest, repetitions] = header;
        let (scenes, remainder) = groups.as_chunks::<GROUP>();
        if !remainder.is_empty()
            || !(2..=MAX_OBSERVATIONS).contains(&scenes.len())
        {
            return Err(String::from("expected 2 to 64 complete scenes"));
        }
        let repeats: NonZeroU32 = text(repetitions)?
            .parse()
            .map_err(|_error| String::from("invalid repetition count"))?;
        let frames = u32::try_from(scenes.len())
            .ok()
            .and_then(|count| count.checked_mul(repeats.get()))
            .filter(|count| *count <= MAX_PRESENTATIONS)
            .and_then(NonZeroU32::new)
            .ok_or_else(|| String::from("scene presentation limit exceeded"))?;
        let mut paths = Vec::new();
        paths
            .try_reserve_exact(scenes.len())
            .map_err(|_error| String::from("scene path allocation failed"))?;
        for scene in scenes {
            let arguments = [parent.clone(), digest.clone()]
                .into_iter()
                .chain(scene.iter().cloned())
                .collect();
            paths.push(Arguments::parse_scene(arguments)?);
        }
        Ok(Self { frames, paths })
    }

    /// Verify all selected scenes and measure finite repeated presentation.
    /// # Errors
    /// Rejects bad inputs, nonmatching scenes or native workload failures.
    pub fn run(&self, machine: Machine) -> Result<(), String> {
        let started = now().map_err(|error| error.to_string())?;
        let path = self
            .paths
            .first()
            .ok_or_else(|| String::from("missing scene paths"))?;
        let bundle = CanonicalBundle::open(
            &path.parent,
            path.digest,
            machine.graphics_limits(),
        )
        .map_err(|error| format!("bundle: {error:?}"))?;
        machine
            .validate_manifest(bundle.manifest())
            .map_err(|error| format!("manifest: {error:?}"))?;
        let payloads = GraphicsPayloads::load(&bundle, true)?;
        let assets = payloads.bind(&bundle)?;
        let raster = machine.background_raster();
        let mut pens = vec![Pixel::default(); raster.pixels()];
        let observations = self.load(machine, &assets, &mut pens)?;
        let mut pixels = vec![[0; 4]; raster.pixels()];
        let layout = native_layout(raster, pixels.as_flattened().len())?;
        let mut window = NativeWindow::open(
            "Changing scene verification",
            layout.width(),
            layout.height(),
            true,
        )
        .map_err(|error| format!("window: {error:?}"))?;
        let renderer = window.renderer_name();
        let mut scenes = observations.iter().cycle();
        let performance = window
            .with_frame(layout, |session| {
                let schedule = Schedule {
                    frames: self.frames,
                    loaded: now().map_err(|error| {
                        RenderError::Backend(error.to_string())
                    })?,
                    started,
                };
                render_benchmark::measure_workload(layout, schedule, || {
                    let observation =
                        scenes.next().ok_or(RenderError::InvalidLength)?;
                    draw(
                        session,
                        (machine, &assets),
                        observation,
                        (&mut pens, &mut pixels),
                    )
                })
            })
            .map_err(|error| format!("sequence: {error:?}"))?;
        let workload =
            format!("{} verified changing scenes", observations.len());
        render_benchmark::report(&performance, &renderer, &workload)
    }
}

type Buffers<'frame> = (&'frame mut [Pixel], &'frame mut [[u8; 4]]);

fn draw(
    session: &mut FrameSession<'_, '_>,
    context: (Machine, &SceneAssets<'_>),
    observation: &Observation,
    buffers: Buffers<'_>,
) -> Result<(), RenderError> {
    let (machine, assets) = context;
    let (pens, pixels) = buffers;
    observation
        .reconstruct(machine, assets, pens)
        .map_err(RenderError::Backend)?;
    for (pixel, pen) in pixels.iter_mut().zip(pens) {
        *pixel = pen
            .rgba(&observation.colors)
            .ok_or(RenderError::InvalidLength)?;
    }
    if !session.present(pixels.as_flattened())? {
        return Err(RenderError::InvalidDimensions);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(groups: usize, repeats: &str) -> Vec<OsString> {
        let mut args = vec![
            OsString::from("parent"),
            OsString::from("0".repeat(64)),
            OsString::from(repeats),
        ];
        for _group in 0..groups {
            args.extend(
                [
                    "ram", "regs", "scroll", "palette", "frame", "objects",
                    "controls",
                ]
                .map(OsString::from),
            );
        }
        args
    }

    #[test]
    fn requires_complete_changing_workload_and_bounds_presentations() {
        assert!(Sequence::parse(&paths(1, "1")).is_err());
        assert!(Sequence::parse(&paths(65, "1")).is_err());
        assert!(Sequence::parse(&paths(2, "0")).is_err());
        assert!(Sequence::parse(&paths(2, "10001")).is_err());
        let mut partial = paths(2, "1");
        let _removed = partial.pop();
        assert!(Sequence::parse(&partial).is_err());
        let result = Sequence::parse(&paths(3, "7"));
        assert!(result.is_ok_and(|sequence| sequence.frames.get() == 21));
    }
}
