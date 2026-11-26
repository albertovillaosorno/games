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
//   - A bounded executable check of title-independent host assumptions.
// - Must-Not:
//   - Implement gameplay rules, render, or access host services.
// - Allows:
//   - Inputs: Synthetic logical controls and elapsed durations.
//   - Outputs: Success or a gameplay invariant failure.
//   - Side effects: None beyond the executable's completion status.
// - Split-When:
//   - Another smoke scenario requires distinct title setup.
// - Merge-When:
//   - A common executable test contract covers equivalent checks.
// - Summary:
//   - Exercise the linked portable four-player street simulation.
// - Description:
//   - Verify admission and movement across different frame partitions.
// - Usage:
//   - Run through the fixed contributor validation gate.
// - Defaults:
//   - No original assets or native preview features.

//! Executable deterministic core acceptance scenario.

use std::num::NonZeroU32;

use games::input::LogicalAction;
use games::replay::{InputScript, Scripted};
use games::timing::StepClock;
use games_simpsons_arcade::{
    Action, Cabinet, Control, Participation, Phase, Player, StreetLayout,
    TICK_INTERVAL,
};

const fn change(
    player: Player,
    control: Control,
    down: bool,
) -> LogicalAction<Action> {
    LogicalAction {
        action: Action { control, player },
        down,
    }
}

fn scenario() -> InputScript<LogicalAction<Action>> {
    let mut script = InputScript::new();
    for (tick, player) in (0u64..).zip(Player::ALL) {
        script.record(tick, change(player, Control::Start, true));
        script.record(tick, change(player, Control::Start, false));
        script.record(4, change(player, Control::Right, true));
        script.record(12, change(player, Control::Right, false));
    }
    for tick in [13, 15] {
        script.record(tick, change(Player::Marge, Control::Pause, true));
        script.record(tick, change(Player::Marge, Control::Pause, false));
    }
    script
}

fn main() -> Result<(), &'static str> {
    let cabinet = Cabinet::new(StreetLayout::DOWNTOWN)
        .map_err(|_error| "invalid smoke layout")?;
    let mut single_clock = StepClock::new(TICK_INTERVAL, NonZeroU32::MAX)
        .map_err(|_error| "invalid smoke clock")?;
    let mut split_clock = single_clock;
    let script = scenario();
    let mut single = Scripted::new(cabinet, script.clone());
    let mut split = Scripted::new(cabinet, script);
    let report = single_clock
        .advance(TICK_INTERVAL.saturating_mul(16), &mut single)
        .map_err(|_error| "single-frame advance failed")?;
    for _ in 0u8..16 {
        let step = split_clock
            .advance(TICK_INTERVAL, &mut split)
            .map_err(|_error| "split-frame advance failed")?;
        if step.next_tick == 14 && split.state().phase() != Phase::Paused {
            return Err("pause did not suspend play");
        }
    }
    if report.updates != 16 || single.state() != split.state() {
        return Err("frame partitions changed the simulation");
    }
    if single.state().phase() != Phase::Playing {
        return Err("pause and resume did not restore play");
    }
    for (station, spawn) in single
        .state()
        .stations()
        .into_iter()
        .zip(StreetLayout::DOWNTOWN.spawn)
    {
        if station.participation != Participation::Playing
            || station.position.x != spawn.x.saturating_add(16)
            || station.position.y != spawn.y
        {
            return Err("a station failed admission or movement");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn linked_core_obeys_the_smoke_scenario() -> Result<(), &'static str> {
        super::main()
    }
}
