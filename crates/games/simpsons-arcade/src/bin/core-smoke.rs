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
//   - Bounded executable checks of portable gameplay and engine contracts.
// - Must-Not:
//   - Implement gameplay rules, render, or access host services.
// - Allows:
//   - Inputs: Synthetic controls, durations, pixels and preferences.
//   - Outputs: Success or a portable invariant failure.
//   - Side effects: None beyond the executable's completion status.
// - Split-When:
//   - Another smoke scenario requires distinct title setup.
// - Merge-When:
//   - A common executable test contract covers equivalent checks.
// - Summary:
//   - Exercise the linked portable four-player street simulation.
// - Description:
//   - Verify movement, touch projection, pixel bounds and save integrity.
// - Usage:
//   - Run through the fixed contributor validation gate.
// - Defaults:
//   - No original assets or native preview features.

//! Executable deterministic core acceptance scenarios.

use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};

use games::input::touch::{
    ContactId, ContactPhase, TouchLayout, TouchPoint, TouchZone,
    normalized_boundary,
};
use games::input::{InputEvent, InputMapping, InputRouter, LogicalAction};
use games::presentation::{
    Dimensions, DisplaySettings, PresentationMode, rgba_row,
};
use games::replay::{InputScript, Scripted};
use games::safety::{SafetyError, decode_save, encode_save};
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

fn gameplay() -> Result<(), &'static str> {
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

fn require(condition: bool, message: &'static str) -> Result<(), &'static str> {
    condition.then_some(()).ok_or(message)
}

fn extent(value: u32) -> Result<NonZeroU32, &'static str> {
    NonZeroU32::new(value).ok_or("zero extent")
}

fn pixels() -> Result<(), &'static str> {
    let rgba = [1u8, 2, 3, 4, 5, 6, 7, 8];
    require(
        rgba_row(&rgba, 2, (1, 0), 1) == Some([5u8, 6, 7, 8].as_slice())
            && rgba_row(&rgba, 2, (1, 0), 2).is_none()
            && rgba_row(&rgba, u32::MAX, (0, u32::MAX), 1).is_none(),
        "pixel row admission mismatch",
    )
}

fn projected_panel(
    width: u32,
    height: u32,
) -> Result<TouchPoint, &'static str> {
    let surface = Dimensions::new(extent(width)?, extent(height)?);
    let design = Dimensions::new(extent(16)?, extent(12)?);
    let layout = PresentationMode::Fill
        .layout_with_protected_bottom(surface, design, Some(extent(3)?))
        .ok_or("protected fill rejected")?;
    let panel = layout.protected.ok_or("missing protected panel")?;
    require(
        panel.source.width == design.width.get()
            && panel.source.height == 3
            && panel.source.y == 9,
        "protected panel cropped",
    )?;
    let logical = TouchPoint { x: 32_768, y: 57_344 };
    let physical = logical
        .on_presentation(layout)
        .ok_or("panel centre not visible")?;
    let projected = physical
        .in_presentation(layout)
        .ok_or("visible panel contact rejected")?;
    require(
        projected.x.abs_diff(logical.x) <= 32
            && projected.y.abs_diff(logical.y) <= 32,
        "touch projection changed the logical point",
    )?;
    if panel.destination.x > 0 {
        let bar = TouchPoint { x: 0, y: physical.y };
        require(
            bar.in_presentation(layout).is_none(),
            "touch admitted in protected panel bar",
        )?;
    }
    Ok(projected)
}

const fn contact(id: ContactId, phase: ContactPhase) -> InputEvent {
    InputEvent::Contact { id, phase }
}

fn touch(point: TouchPoint) -> Result<(), &'static str> {
    let boundary = normalized_boundary(9, 12).ok_or("invalid panel")?;
    let zone =
        TouchZone::new(1, TouchPoint { x: 0, y: boundary }, TouchPoint {
            x: u16::MAX,
            y: u16::MAX,
        })
        .ok_or("invalid control zone")?;
    let mut mapping = InputMapping::new();
    let down = change(Player::Marge, Control::Right, true);
    let up = change(Player::Marge, Control::Right, false);
    mapping.bind_touch(1, down.action);
    mapping.set_touch_layout(TouchLayout::new(vec![zone], NonZeroUsize::MIN));
    let mut router = InputRouter::new(mapping);
    let first = ContactId { device: 1, finger: 1 };
    let second = ContactId { device: 1, finger: 2 };
    require(
        router.handle(contact(first, ContactPhase::Down(point)))
            == [down.clone()]
            && router
                .handle(contact(first, ContactPhase::Down(point)))
                .is_empty()
            && router
                .handle(contact(second, ContactPhase::Down(point)))
                .is_empty()
            && router.handle(InputEvent::TouchCancelled) == [up.clone()]
            && router
                .handle(contact(first, ContactPhase::Motion(Some(point))))
                .is_empty()
            && router.handle(contact(second, ContactPhase::Down(point)))
                == [down]
            && router.handle(contact(second, ContactPhase::Up)) == [up],
        "touch lifetime or admission mismatch",
    )
}

fn save() -> Result<(), &'static str> {
    let maximum = NonZeroU64::new(64).ok_or("zero save budget")?;
    let settings = DisplaySettings {
        presentation: PresentationMode::Fill,
        ..DisplaySettings::default()
    };
    let payload = settings.encode();
    let mut encoded = encode_save(payload.as_bytes(), maximum)
        .map_err(|_error| "preference save rejected")?;
    let verified = decode_save(&encoded, maximum)
        .map_err(|_error| "valid save rejected")?;
    require(
        DisplaySettings::decode(verified) == Ok(settings),
        "preference round trip changed settings",
    )?;
    let last = encoded.last_mut().ok_or("empty save envelope")?;
    *last = 0;
    require(
        decode_save(&encoded, maximum) == Err(SafetyError::InvalidSave)
            && encode_save(&[0; 65], maximum) == Err(SafetyError::TooLarge),
        "corrupt or oversized save admitted",
    )
}

fn main() -> Result<(), &'static str> {
    gameplay()?;
    pixels()?;
    for (width, height) in [(40, 12), (12, 40)] {
        touch(projected_panel(width, height)?)?;
    }
    save()
}

#[cfg(test)]
mod tests {
    #[test]
    fn linked_core_obeys_the_smoke_scenarios() -> Result<(), &'static str> {
        super::main()
    }
}
