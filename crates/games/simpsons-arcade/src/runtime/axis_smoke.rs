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
//   - Finite synthetic admission checks for loaded native axis profiles.
// - Must-Not:
//   - Open hardware, mutate preferences or advance gameplay.
// - Allows:
//   - Inputs: Admitted title profiles and standard physical axis identities.
//   - Outputs: Verified direction count or explicit routing failure.
//   - Side effects: Temporary local controller assignments and input holds.
// - Split-When:
//   - Native physical device acceptance needs an independent workflow.
// - Merge-When:
//   - Other title smoke duplicates generic loaded-profile verification.
// - Summary:
//   - Verify every station's signed-axis press and release thresholds.
// - Description:
//   - Exercise neutral, pre-threshold, press, hold and release samples.
// - Usage:
//   - Run with the exact profiles admitted at native launch.
// - Defaults:
//   - Synthetic events only; no physical device acceptance is implied.

//! Finite native smoke for configured signed controller axes.

use games::input::physical::ControllerAxis;
use games::input::profile::AxisProfile;
use games::input::{InputEvent, LogicalAction};
use games_platform::ir::frame::RenderError;

use super::{Action, CONTROLS, Controls, Player, Profiles, failure};

#[derive(Clone, Copy, Debug)]
struct AxisProbe {
    axis: ControllerAxis,
    device: u64,
    player: Player,
    profile: AxisProfile,
}

impl AxisProbe {
    fn action(self, negative: bool) -> Result<Option<Action>, RenderError> {
        let slot = if negative {
            self.profile.negative
        } else {
            self.profile.positive
        };
        slot.map(|index| {
            let control =
                *CONTROLS.get(index).ok_or(RenderError::InvalidSprite)?;
            Ok(Action {
                control,
                player: self.player,
            })
        })
        .transpose()
    }

    fn check(
        self,
        controls: &mut Controls,
        negative: bool,
    ) -> Result<(), RenderError> {
        let binding = self.action(negative)?;
        let press = self.profile.press;
        let release = self.profile.release;
        for (magnitude, edge) in [
            (0, None),
            (press.saturating_sub(1), None),
            (press, Some(true)),
            (press, None),
            (release.saturating_add(1), None),
            (release, Some(false)),
            (0, None),
        ] {
            let sample = i16::try_from(magnitude).map_err(failure)?;
            let value = if negative {
                sample.saturating_neg()
            } else {
                sample
            };
            let observed = controls.actions(InputEvent::Axis {
                code: self.axis.code(),
                device: self.device,
                value,
            })?;
            let expected = binding
                .zip(edge)
                .map(|(action, down)| LogicalAction { action, down });
            if observed.as_slice() != expected.as_slice() {
                return Err(failure(format!(
                    concat!(
                        "axis {:?} player {:?} negative {} magnitude {}",
                        ": expected {:?}, got {:?}"
                    ),
                    self.axis,
                    self.player,
                    negative,
                    magnitude,
                    expected,
                    observed
                )));
            }
        }
        Ok(())
    }
}

pub(super) fn verify(profiles: &Profiles) -> Result<usize, RenderError> {
    let mut controls = Controls::with_profiles(false, profiles)?;
    let mut checked = 0usize;
    for (player, axes) in Player::ALL.into_iter().zip(profiles.axes) {
        let device = u64::try_from(player.index())
            .map_err(failure)?
            .saturating_add(1);
        if !controls
            .actions(InputEvent::Attached { device })?
            .is_empty()
        {
            return Err(failure("axis smoke attached a held controller"));
        }
        for (axis, profile) in ControllerAxis::ALL.into_iter().zip(axes) {
            let probe = AxisProbe {
                axis,
                device,
                player,
                profile,
            };
            for negative in [true, false] {
                probe.check(&mut controls, negative)?;
                checked = checked.saturating_add(1);
            }
        }
    }
    Ok(checked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::configured_profiles;

    #[test]
    fn all_loaded_signed_axis_directions_and_thresholds_are_checked()
    -> Result<(), RenderError> {
        let profiles = configured_profiles(concat!(
            "version=1\n",
            "axis 2 rightx=right/left\n",
            "axis-threshold 2 rightx=32767/32766\n",
            "axis 3 lefttrigger=none/primary\n",
            "axis-threshold 3 lefttrigger=1/0\n",
            "axis 4 leftx=right/right\n",
        ))
        .map_err(failure)?;
        if verify(&profiles)? != 48 {
            return Err(failure("axis smoke did not cover four full profiles"));
        }
        Ok(())
    }
}
