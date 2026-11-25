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
//   - Public keyboard, controller and configurable touch input tests.
// - Must-Not:
//   - Require a physical controller or native window system.
// - Allows:
//   - Inputs: Synthetic platform-neutral device events.
//   - Outputs: Observable pressed and released logical actions.
//   - Side effects: None outside in-memory input router.
// - Split-When:
//   - Native adapters acquire platform-specific integration fixtures.
// - Merge-When:
//   - Shared input tests cover the same behaviour.
// - Summary:
//   - Keyboard, gamepad, touch and hot-plug regression scenarios.
// - Description:
//   - Verifies aggregation and disconnection without stuck controls.
// - Usage:
//   - cargo validate
// - Defaults:
//   - No external devices or title assets.
//

use super::{InputEvent, InputMapping, InputRouter, LogicalAction};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Action {
    Confirm,
    Jump,
}

#[test]
fn key_and_gamepad_share_one_logical_hold() {
    let mut map = InputMapping::new();
    map.bind_key(42, Action::Jump);
    map.bind_button(7, Action::Jump);
    let mut router = InputRouter::new(map);
    let down = LogicalAction {
        action: Action::Jump,
        down: true,
    };
    let up = LogicalAction {
        action: Action::Jump,
        down: false,
    };
    assert_eq!(router.handle(InputEvent::Key { code: 42, down: true }), [
        down
    ]);
    assert_eq!(router.handle(InputEvent::Attached { device: 5 }), []);
    assert_eq!(
        router.handle(InputEvent::Button {
            device: 5,
            code: 7,
            down: true,
        }),
        []
    );
    assert_eq!(router.handle(InputEvent::Key { code: 42, down: false }), []);
    assert_eq!(router.handle(InputEvent::Detached { device: 5 }), [up]);
}

#[test]
fn hotplug_releases_only_disconnected_controller() {
    let mut map = InputMapping::new();
    map.bind_button(1, Action::Confirm);
    let mut router = InputRouter::new(map);
    let pressed = LogicalAction {
        action: Action::Confirm,
        down: true,
    };
    let released = LogicalAction {
        action: Action::Confirm,
        down: false,
    };
    assert_eq!(
        router.handle(InputEvent::Button {
            device: 1,
            code: 1,
            down: true,
        }),
        []
    );
    assert_eq!(router.handle(InputEvent::Attached { device: 1 }), []);
    assert_eq!(router.handle(InputEvent::Attached { device: 2 }), []);
    assert_eq!(
        router.handle(InputEvent::Button {
            device: 1,
            code: 1,
            down: true,
        }),
        [pressed]
    );
    assert_eq!(
        router.handle(InputEvent::Button {
            device: 2,
            code: 1,
            down: true,
        }),
        []
    );
    assert_eq!(router.handle(InputEvent::Detached { device: 1 }), []);
    assert_eq!(
        router.handle(InputEvent::Button {
            device: 2,
            code: 1,
            down: false,
        }),
        [released]
    );
}

#[test]
fn title_specific_touch_zones_remain_independent() {
    let mut first_map = InputMapping::new();
    first_map.bind_touch(8, Action::Jump);
    let mut second_map = InputMapping::new();
    second_map.bind_touch(8, Action::Confirm);
    let mut first = InputRouter::new(first_map);
    let mut second = InputRouter::new(second_map);
    assert_eq!(first.handle(InputEvent::Touch { zone: 8, down: true }), [
        LogicalAction {
            action: Action::Jump,
            down: true
        }
    ]);
    assert_eq!(second.handle(InputEvent::Touch { zone: 8, down: true }), [
        LogicalAction {
            action: Action::Confirm,
            down: true
        }
    ]);
    assert_eq!(first.handle(InputEvent::Touch { zone: 8, down: true }), []);
    assert_eq!(first.handle(InputEvent::Touch { zone: 8, down: false }), [
        LogicalAction {
            action: Action::Jump,
            down: false
        }
    ]);
}
