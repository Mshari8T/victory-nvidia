//! Gamepads that XInput does not list, read over HID: Sony's DualShock 4
//! and DualSense (USB and Bluetooth, with rumble), the Switch 2 Pro
//! Controller (USB), and any other HID gamepad or joystick by its usages.
//! XInput devices (paths with `IG_`) are left to XInput.
//!
//! The Sony report layouts are the ones Linux's `hid-playstation` and
//! `hid-sony` drivers read and write; the Switch 2 ones and its setup
//! commands are SDL's.

use super::{
    A, B, DOWN, LEFT, LEFT_SHOULDER, LEFT_STICK, MENU, PadState, RIGHT, RIGHT_SHOULDER,
    RIGHT_STICK, UP, VIEW, X, Y,
};

const SONY: u16 = 0x054C;

/// A pad whose reports we read ourselves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Sony {
    DualShock4,
    DualSense,
}

impl Sony {
    pub(super) fn of(vendor: u16, product: u16) -> Option<Sony> {
        match (vendor, product) {
            // Both revisions, and the wireless adapter.
            (SONY, 0x05C4 | 0x09CC | 0x0BA0) => Some(Sony::DualShock4),
            // DualSense and DualSense Edge.
            (SONY, 0x0CE6 | 0x0DF2) => Some(Sony::DualSense),
            _ => None,
        }
    }

    /// The feature report that switches a pad on Bluetooth from its short
    /// reports to the full ones.
    pub(super) fn full_reports_feature(self) -> u8 {
        match self {
            Sony::DualShock4 => 0x02,
            Sony::DualSense => 0x05,
        }
    }
}

/// The D-pad of a hat switch: 0 up, then clockwise to 7 (up-left); anything
/// else is the centre.
fn hat(value: u8) -> u32 {
    match value {
        0 => UP,
        1 => UP | RIGHT,
        2 => RIGHT,
        3 => RIGHT | DOWN,
        4 => DOWN,
        5 => DOWN | LEFT,
        6 => LEFT,
        7 => LEFT | UP,
        _ => 0,
    }
}

/// A stick byte (128 at rest) to -1..1.
fn axis(value: u8) -> f32 {
    ((f32::from(value) - 128.0) / 127.0).clamp(-1.0, 1.0)
}

/// Sony's three button bytes, by position: square, cross, circle and
/// triangle as the Xbox pad's X, A, B and Y; Options as Menu; Share
/// (Create) and the touchpad's click as View.
fn sony_buttons(first: u8, second: u8, third: u8) -> u32 {
    let mut buttons = hat(first & 0x0F);
    for (bit, button) in [(0x10, X), (0x20, A), (0x40, B), (0x80, Y)] {
        if first & bit != 0 {
            buttons |= button;
        }
    }
    for (bit, button) in [
        (0x01, LEFT_SHOULDER),
        (0x02, RIGHT_SHOULDER),
        (0x10, VIEW),
        (0x20, MENU),
        (0x40, LEFT_STICK),
        (0x80, RIGHT_STICK),
    ] {
        if second & bit != 0 {
            buttons |= button;
        }
    }
    if third & 0x02 != 0 {
        buttons |= VIEW;
    }
    buttons
}

/// Sticks first, then the three button bytes, then the triggers: the
/// DualShock 4's reports and the DualSense's short Bluetooth one. `at` is
/// the left stick's offset.
fn buttons_then_triggers(r: &[u8], at: usize) -> Option<PadState> {
    let r = r.get(at..at + 9)?;
    Some(PadState {
        buttons: sony_buttons(r[4], r[5], r[6]),
        left_trigger: f32::from(r[7]) / 255.0,
        right_trigger: f32::from(r[8]) / 255.0,
        left_x: axis(r[0]),
        left_y: -axis(r[1]),
        right_x: axis(r[2]),
        right_y: -axis(r[3]),
    })
}

/// Sticks, triggers, a counter, then the button bytes: the DualSense's
/// full reports. `at` is the left stick's offset.
fn triggers_then_buttons(r: &[u8], at: usize) -> Option<PadState> {
    let r = r.get(at..at + 10)?;
    Some(PadState {
        buttons: sony_buttons(r[7], r[8], r[9]),
        left_trigger: f32::from(r[4]) / 255.0,
        right_trigger: f32::from(r[5]) / 255.0,
        left_x: axis(r[0]),
        left_y: -axis(r[1]),
        right_x: axis(r[2]),
        right_y: -axis(r[3]),
    })
}

/// The pad's state in an input report (report id first), or `None` for a
/// report that carries none.
pub(super) fn sony_report(pad: Sony, r: &[u8]) -> Option<PadState> {
    match (pad, *r.first()?) {
        // USB, 64 bytes.
        (Sony::DualSense, 0x01) if r.len() >= 64 => triggers_then_buttons(r, 1),
        // Bluetooth before the full reports are asked for (10 bytes).
        (Sony::DualSense, 0x01) => buttons_then_triggers(r, 1),
        // Bluetooth, full.
        (Sony::DualSense, 0x31) => triggers_then_buttons(r, 2),
        // USB, and Bluetooth before the full reports.
        (Sony::DualShock4, 0x01) => buttons_then_triggers(r, 1),
        // Bluetooth, full.
        (Sony::DualShock4, 0x11) => buttons_then_triggers(r, 3),
        _ => None,
    }
}

/// CRC-32 (IEEE) as Sony's Bluetooth output reports carry it.
fn crc32(parts: &[&[u8]]) -> u32 {
    let mut crc = !0u32;
    for byte in parts.iter().flat_map(|p| p.iter()) {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

/// The output report that sets the motors (`low` the left, low-frequency
/// one; `high` the right), `length` bytes as the device declares its
/// output reports. Lights are left as they are.
pub(super) fn sony_rumble(
    pad: Sony,
    bluetooth: bool,
    length: usize,
    low: u16,
    high: u16,
) -> Vec<u8> {
    let (low, high) = ((low >> 8) as u8, (high >> 8) as u8);
    let mut r = vec![0u8; length.max(if bluetooth { 78 } else { 32 })];
    // Where the common part (flags, then the right and left motors) starts.
    let common = match (pad, bluetooth) {
        (Sony::DualSense, false) => {
            r[0] = 0x02;
            1
        }
        (Sony::DualSense, true) => {
            r[0] = 0x31;
            r[1] = 0x00; // sequence 0
            r[2] = 0x10; // the tag Sony's driver sends
            3
        }
        (Sony::DualShock4, false) => {
            r[0] = 0x05;
            1
        }
        (Sony::DualShock4, true) => {
            r[0] = 0x11;
            r[1] = 0xC0; // HID report, with a CRC
            3
        }
    };
    // DualSense: compatible vibration and haptics select. DualShock 4: the
    // motors only (not the light bar).
    r[common] = match pad {
        Sony::DualSense => 0x03,
        Sony::DualShock4 => 0x01,
    };
    let motors = match pad {
        Sony::DualSense => common + 2,
        Sony::DualShock4 => common + 3,
    };
    r[motors] = high;
    r[motors + 1] = low;
    if bluetooth {
        let end = r.len() - 4;
        let crc = crc32(&[&[0xA2], &r[..end]]);
        r[end..].copy_from_slice(&crc.to_le_bytes());
    }
    r
}

const NINTENDO: u16 = 0x057E;

/// The Switch 2 Pro Controller. Other Switch 2 pads (Joy-Con, the GameCube
/// pad) have layouts of their own and are not read yet.
pub(super) fn is_switch2_pro(vendor: u16, product: u16) -> bool {
    (vendor, product) == (NINTENDO, 0x2069)
}

/// One stick axis as the pad's flash calibrates it: the value at rest and
/// how far it travels above and below that.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Travel {
    pub neutral: u16,
    pub above: u16,
    pub below: u16,
}

/// Left x, left y, right x, right y.
pub(super) type Calibration = [Option<Travel>; 4];

/// Two 12-bit values packed in three bytes.
fn twelve(d: &[u8]) -> (u16, u16) {
    (
        u16::from(d[0]) | u16::from(d[1] & 0x0F) << 8,
        u16::from(d[1] >> 4) | u16::from(d[2]) << 4,
    )
}

/// A stick's calibration block: the rest values, the travel above, the
/// travel below, each as an x and y pair.
pub(super) fn stick_calibration(d: &[u8]) -> Option<[Travel; 2]> {
    let d = d.get(..9)?;
    let ((nx, ny), (ax, ay), (bx, by)) = (twelve(&d[0..]), twelve(&d[3..]), twelve(&d[6..]));
    let travel = |neutral, above, below| Travel {
        neutral,
        above,
        below,
    };
    Some([travel(nx, ax, bx), travel(ny, ay, by)])
}

/// A 12-bit stick value to -1..1, larger up and right; without a
/// calibration, 2048 is the centre.
fn switch2_axis(value: u16, travel: Option<Travel>) -> f32 {
    match travel {
        Some(t) if t.neutral != 0 && t.above != 0 && t.below != 0 => {
            let v = f32::from(value) - f32::from(t.neutral);
            let reach = if v < 0.0 { t.below } else { t.above };
            (v / f32::from(reach)).clamp(-1.0, 1.0)
        }
        _ => normalise(i64::from(value), 0, 4096),
    }
}

fn held(byte: u8, bits: &[(u8, u32)]) -> u32 {
    bits.iter()
        .filter(|(bit, _)| byte & bit != 0)
        .fold(0, |all, (_, button)| all | button)
}

/// The Switch 2 Pro Controller's state in an input report: 0x09, the one
/// it sends once we set it up, or 0x05, the one Steam asks for. Buttons go
/// by position, as on the Xbox pad: the bottom one (Nintendo's B) is A, the
/// right one (Nintendo's A) is B, and so on; Plus is Menu, Minus is View.
/// The triggers are buttons, so 0 or 1.
pub(super) fn switch2_report(r: &[u8], cal: &Calibration) -> Option<PadState> {
    let (buttons, triggers, sticks) = match *r.first()? {
        0x09 => {
            let r = r.get(..12)?;
            let buttons = held(
                r[3],
                &[
                    (0x01, A),
                    (0x02, B),
                    (0x04, X),
                    (0x08, Y),
                    (0x10, RIGHT_SHOULDER),
                    (0x40, MENU),
                    (0x80, RIGHT_STICK),
                ],
            ) | held(
                r[4],
                &[
                    (0x01, DOWN),
                    (0x02, RIGHT),
                    (0x04, LEFT),
                    (0x08, UP),
                    (0x10, LEFT_SHOULDER),
                    (0x40, VIEW),
                    (0x80, LEFT_STICK),
                ],
            );
            (buttons, (r[4] & 0x20, r[3] & 0x20), &r[6..12])
        }
        0x05 => {
            let r = r.get(..17)?;
            let buttons = held(
                r[5],
                &[
                    (0x01, X),
                    (0x02, Y),
                    (0x04, A),
                    (0x08, B),
                    (0x40, RIGHT_SHOULDER),
                ],
            ) | held(
                r[6],
                &[
                    (0x01, VIEW),
                    (0x02, MENU),
                    (0x04, RIGHT_STICK),
                    (0x08, LEFT_STICK),
                ],
            ) | held(
                r[7],
                &[
                    (0x01, DOWN),
                    (0x02, UP),
                    (0x04, RIGHT),
                    (0x08, LEFT),
                    (0x40, LEFT_SHOULDER),
                ],
            );
            (buttons, (r[7] & 0x80, r[5] & 0x80), &r[11..17])
        }
        _ => return None,
    };
    let (lx, ly) = twelve(&sticks[0..]);
    let (rx, ry) = twelve(&sticks[3..]);
    let pressed = |bit: u8| if bit != 0 { 1.0 } else { 0.0 };
    Some(PadState {
        buttons,
        left_trigger: pressed(triggers.0),
        right_trigger: pressed(triggers.1),
        left_x: switch2_axis(lx, cal[0]),
        left_y: switch2_axis(ly, cal[1]),
        right_x: switch2_axis(rx, cal[2]),
        right_y: switch2_axis(ry, cal[3]),
    })
}

/// Generic Desktop usages a gamepad's axes and hat use.
pub(super) const USAGE_X: u16 = 0x30;
pub(super) const USAGE_HAT: u16 = 0x39;
/// The six axes, X to Rz.
pub(super) const AXES: usize = 6;

/// What a generic HID pad reports: its axes (X, Y, Z, Rx, Ry, Rz) at -1..1
/// where present, the hat's direction, the buttons held (HID button
/// numbers, from 1).
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Generic {
    pub axes: [Option<f32>; AXES],
    pub hat: Option<u8>,
    pub buttons: Vec<u16>,
}

/// HID button numbers for A, B, X, Y, the shoulders, the triggers, View,
/// Menu and the stick clicks, in that order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ButtonMap(pub [u16; 12]);

impl ButtonMap {
    /// The layout most DirectInput pads use: 1 west (X), 2 south (A),
    /// 3 east (B), 4 north (Y), 5-8 shoulders and triggers, 9 select,
    /// 10 start, 11-12 the sticks.
    pub const DIRECTINPUT: ButtonMap = ButtonMap([2, 3, 1, 4, 5, 6, 7, 8, 9, 10, 11, 12]);

    /// `VICTORY_PAD_BUTTONS`: twelve button numbers, comma-separated, in
    /// the order above; anything else keeps the default.
    pub fn parse(text: Option<&str>) -> ButtonMap {
        let numbers: Option<Vec<u16>> = text.map(|t| {
            t.split(',')
                .map(|n| n.trim().parse().ok())
                .collect::<Option<Vec<u16>>>()
                .unwrap_or_default()
        });
        match numbers.and_then(|n| <[u16; 12]>::try_from(n).ok()) {
            Some(numbers) => ButtonMap(numbers),
            None => ButtonMap::DIRECTINPUT,
        }
    }
}

/// A generic pad as the console's: X and Y the left stick; Z and Rz the
/// right one when both are there (then Rx and Ry, if there, are the
/// triggers), else Rx and Ry; the hat as the D-pad.
pub(super) fn generic(g: &Generic, map: &ButtonMap) -> PadState {
    let [x, y, z, rx, ry, rz] = g.axes;
    let held = |n: u16| g.buttons.contains(&n);
    let mut buttons = g.hat.map_or(0, hat);
    let bits = [
        A,
        B,
        X,
        Y,
        LEFT_SHOULDER,
        RIGHT_SHOULDER,
        0,
        0,
        VIEW,
        MENU,
        LEFT_STICK,
        RIGHT_STICK,
    ];
    for (n, bit) in map.0.iter().zip(bits) {
        if held(*n) {
            buttons |= bit;
        }
    }
    let ((right_x, right_y), triggers) = match (z, rz) {
        (Some(z), Some(rz)) => ((Some(z), Some(rz)), rx.zip(ry)),
        _ => ((rx, ry), None),
    };
    let pressed = |n: u16| if held(n) { 1.0 } else { 0.0 };
    let (left_trigger, right_trigger) = match triggers {
        Some((l, r)) => ((l + 1.0) / 2.0, (r + 1.0) / 2.0),
        None => (pressed(map.0[6]), pressed(map.0[7])),
    };
    PadState {
        buttons,
        left_trigger: left_trigger.max(pressed(map.0[6])),
        right_trigger: right_trigger.max(pressed(map.0[7])),
        left_x: x.unwrap_or(0.0),
        left_y: -y.unwrap_or(0.0),
        right_x: right_x.unwrap_or(0.0),
        right_y: -right_y.unwrap_or(0.0),
    }
}

/// A value in `min..=max` to -1..1 (HID reports down and right as larger).
pub(super) fn normalise(value: i64, min: i64, max: i64) -> f32 {
    if max <= min {
        return 0.0;
    }
    ((2.0 * (value - min) as f64 / (max - min) as f64 - 1.0) as f32).clamp(-1.0, 1.0)
}

#[cfg(windows)]
pub(super) use host::Pads;

#[cfg(windows)]
mod host {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
        CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CM_Get_Device_Interface_List_SizeW,
        CM_Get_Device_Interface_ListW, CR_SUCCESS, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT,
        SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
        SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
        SetupDiGetDeviceInterfaceDetailW,
    };
    use windows_sys::Win32::Devices::HumanInterfaceDevice::{
        HIDD_ATTRIBUTES, HIDP_CAPS, HIDP_STATUS_SUCCESS, HIDP_VALUE_CAPS, HidD_FreePreparsedData,
        HidD_GetAttributes, HidD_GetFeature, HidD_GetHidGuid, HidD_GetPreparsedData, HidP_GetCaps,
        HidP_GetUsageValue, HidP_GetUsages, HidP_GetValueCaps, HidP_Input, PHIDP_PREPARSED_DATA,
    };
    use windows_sys::Win32::Devices::Usb::{
        PIPE_TRANSFER_TIMEOUT, UsbdPipeTypeBulk, WINUSB_INTERFACE_HANDLE, WINUSB_PIPE_INFORMATION,
        WinUsb_Free, WinUsb_Initialize, WinUsb_QueryPipe, WinUsb_ReadPipe, WinUsb_SetPipePolicy,
        WinUsb_WritePipe,
    };
    use windows_sys::Win32::Foundation::{
        CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        ReadFile, WriteFile,
    };
    use windows_sys::core::GUID;

    use super::*;

    /// How often new pads are looked for.
    const RESCAN: Duration = Duration::from_secs(2);
    /// Generic Desktop usages of a joystick and a gamepad.
    const JOYSTICK: u16 = 0x04;
    const GAMEPAD: u16 = 0x05;
    /// HID's button usage page.
    const BUTTONS: u16 = 0x09;

    /// An open handle, closed when dropped.
    struct Handle(HANDLE);

    // SAFETY: a file handle may be used from any thread.
    unsafe impl Send for Handle {}
    // SAFETY: as above; reads and writes go through different handles.
    unsafe impl Sync for Handle {}

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: our own handle, closed once.
            unsafe { CloseHandle(self.0) };
        }
    }

    impl Handle {
        fn open(path: &[u16], access: u32) -> Option<Handle> {
            Handle::open_with(path, access, 0)
        }

        fn open_with(path: &[u16], access: u32, flags: u32) -> Option<Handle> {
            // SAFETY: a NUL-terminated path.
            let h = unsafe {
                CreateFileW(
                    path.as_ptr(),
                    access,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    flags,
                    std::ptr::null_mut(),
                )
            };
            (h != INVALID_HANDLE_VALUE).then_some(Handle(h))
        }
    }

    /// Preparsed data, freed when dropped.
    struct Preparsed(PHIDP_PREPARSED_DATA);

    // SAFETY: read-only data the HID parser reads from any thread.
    unsafe impl Send for Preparsed {}

    impl Drop for Preparsed {
        fn drop(&mut self) {
            // SAFETY: our own preparsed data, freed once.
            unsafe { HidD_FreePreparsedData(self.0) };
        }
    }

    /// How a pad's reports are read.
    enum Reader {
        Sony(Sony),
        /// The command interface stays open with the pad: it may stop
        /// reporting once that is let go.
        Switch2 {
            cal: Calibration,
            _commands: Option<Commands>,
        },
        /// The axes present (index in X..Rz) and their ranges.
        Generic {
            preparsed: Preparsed,
            axes: Vec<(usize, i64, i64)>,
            hat: Option<(i64, i64)>,
            map: ButtonMap,
        },
    }

    impl Reader {
        fn read(&self, report: &mut [u8]) -> Option<PadState> {
            match self {
                Reader::Sony(pad) => sony_report(*pad, report),
                Reader::Switch2 { cal, .. } => switch2_report(report, cal),
                Reader::Generic {
                    preparsed,
                    axes,
                    hat,
                    map,
                } => {
                    let value = |usage: u16| -> Option<i64> {
                        let mut v = 0u32;
                        // SAFETY: the report and preparsed data are this
                        // device's; the value is written to `v`.
                        let status = unsafe {
                            HidP_GetUsageValue(
                                HidP_Input,
                                0x01,
                                0,
                                usage,
                                &mut v,
                                preparsed.0,
                                report.as_ptr(),
                                report.len() as u32,
                            )
                        };
                        (status == HIDP_STATUS_SUCCESS).then_some(i64::from(v))
                    };
                    let mut g = Generic::default();
                    for &(k, min, max) in axes {
                        g.axes[k] = value(USAGE_X + k as u16).map(|v| normalise(v, min, max));
                    }
                    g.hat = hat.and_then(|(min, max)| {
                        let v = value(USAGE_HAT)?;
                        (min..=max).contains(&v).then(|| (v - min) as u8)
                    });
                    let mut usages = [0u16; 64];
                    let mut count = usages.len() as u32;
                    // SAFETY: room for `count` usages; the report is this
                    // device's.
                    let status = unsafe {
                        HidP_GetUsages(
                            HidP_Input,
                            BUTTONS,
                            0,
                            usages.as_mut_ptr(),
                            &mut count,
                            preparsed.0,
                            report.as_mut_ptr(),
                            report.len() as u32,
                        )
                    };
                    if status == HIDP_STATUS_SUCCESS {
                        g.buttons = usages[..count as usize].to_vec();
                    }
                    Some(generic(&g, map))
                }
            }
        }
    }

    /// One pad: its latest state while connected.
    struct Pad {
        id: u64,
        path: Vec<u16>,
        state: Mutex<Option<PadState>>,
        connected: AtomicBool,
        /// Sony pads: the handle output reports go through (reads block
        /// the other), whether on Bluetooth, the output report's length,
        /// and the last motor speeds sent.
        output: Option<(Sony, Handle, bool, usize)>,
        rumble: Mutex<(u16, u16)>,
    }

    /// The HID pads found so far, in the order they were connected.
    #[derive(Default)]
    pub(in super::super) struct Pads {
        pads: Arc<Mutex<Vec<Arc<Pad>>>>,
        started: AtomicBool,
    }

    impl Pads {
        /// The connected pads' ids and states, oldest first; starts looking
        /// for pads on first use.
        pub fn connected(&self) -> Vec<(u64, PadState)> {
            if !self.started.swap(true, Ordering::Relaxed) {
                let pads = Arc::clone(&self.pads);
                let spawned = std::thread::Builder::new()
                    .name("victory hid pads".into())
                    .spawn(move || scan(&pads));
                if let Err(e) = spawned {
                    tracing::warn!(error = %e, "no HID pads: the thread looking for them could not start");
                }
            }
            let pads = super::super::lock(&self.pads);
            pads.iter()
                .filter(|p| p.connected.load(Ordering::Relaxed))
                .filter_map(|p| Some((p.id, (*super::super::lock(&p.state))?)))
                .collect()
        }

        /// Sets pad `id`'s motors, if it has any we can drive.
        pub fn rumble(&self, id: u64, low: u16, high: u16) {
            let pad = super::super::lock(&self.pads)
                .iter()
                .find(|p| p.id == id)
                .cloned();
            let Some(pad) = pad else { return };
            let Some((sony, handle, bluetooth, length)) = &pad.output else {
                return;
            };
            let mut last = super::super::lock(&pad.rumble);
            if *last == (low, high) {
                return;
            }
            *last = (low, high);
            let report = sony_rumble(*sony, *bluetooth, *length, low, high);
            let mut written = 0;
            // SAFETY: the report outlives the call; a failed write (the pad
            // went away) is noticed by its reader.
            unsafe {
                WriteFile(
                    handle.0,
                    report.as_ptr(),
                    report.len() as u32,
                    &mut written,
                    std::ptr::null_mut(),
                )
            };
        }
    }

    /// Looks for new pads for as long as the process runs.
    fn scan(pads: &Mutex<Vec<Arc<Pad>>>) {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let map = ButtonMap::parse(std::env::var("VICTORY_PAD_BUTTONS").ok().as_deref());
        loop {
            for path in hid_paths() {
                let known = super::super::lock(pads)
                    .iter()
                    .any(|p| p.path == path && p.connected.load(Ordering::Relaxed));
                if known || is_xinput(&path) {
                    continue;
                }
                let Some((pad, reader, input, read)) =
                    open(&path, NEXT.load(Ordering::Relaxed), map)
                else {
                    continue;
                };
                NEXT.fetch_add(1, Ordering::Relaxed);
                let pad = Arc::new(pad);
                let mut list = super::super::lock(pads);
                list.retain(|p| p.connected.load(Ordering::Relaxed));
                list.push(Arc::clone(&pad));
                drop(list);
                let spawned = std::thread::Builder::new()
                    .name("victory hid pad".into())
                    .spawn(move || listen(&pad, &reader, input, &read));
                if let Err(e) = spawned {
                    tracing::warn!(error = %e, "a HID pad could not be read");
                }
            }
            std::thread::sleep(RESCAN);
        }
    }

    /// XInput's own HID devices (`IG_` in the path): XInput reads them.
    fn is_xinput(path: &[u16]) -> bool {
        String::from_utf16_lossy(path)
            .to_ascii_lowercase()
            .contains("ig_")
    }

    /// Opens the device at `path` if it is a gamepad or joystick: the pad,
    /// how to read it, its input report length and the handle to read.
    fn open(path: &[u16], id: u64, map: ButtonMap) -> Option<(Pad, Reader, usize, Handle)> {
        let read = Handle::open(path, GENERIC_READ | GENERIC_WRITE)
            .or_else(|| Handle::open(path, GENERIC_READ))?;
        let mut attributes = HIDD_ATTRIBUTES {
            Size: size_of::<HIDD_ATTRIBUTES>() as u32,
            ..Default::default()
        };
        let mut raw: PHIDP_PREPARSED_DATA = 0;
        // SAFETY: the HID handle is open and both out pointers are valid.
        if !unsafe {
            HidD_GetAttributes(read.0, &mut attributes)
                && HidD_GetPreparsedData(read.0, &mut raw)
        } {
            return None;
        }
        let preparsed = Preparsed(raw);
        let mut caps = HIDP_CAPS::default();
        // SAFETY: this device's preparsed data.
        if unsafe { HidP_GetCaps(preparsed.0, &mut caps) } != HIDP_STATUS_SUCCESS
            || caps.UsagePage != 0x01
            || !matches!(caps.Usage, JOYSTICK | GAMEPAD)
        {
            return None;
        }
        let name = String::from_utf16_lossy(path).to_ascii_lowercase();
        let (reader, output) = match Sony::of(attributes.VendorID, attributes.ProductID) {
            None if is_switch2_pro(attributes.VendorID, attributes.ProductID) => {
                let (cal, commands) = match Commands::open() {
                    Some(c) => (c.set_up(), Some(c)),
                    None => (Calibration::default(), None),
                };
                (
                    Reader::Switch2 {
                        cal,
                        _commands: commands,
                    },
                    None,
                )
            }
            Some(sony) => {
                // The Bluetooth HID service's id in the path, or the long
                // Bluetooth reports.
                let bluetooth = name.contains("00001124-0000-1000-8000-00805f9b34fb")
                    || caps.InputReportByteLength > 64;
                if bluetooth {
                    let mut feature = [0u8; 64];
                    feature[0] = sony.full_reports_feature();
                    // SAFETY: room for the report.
                    unsafe {
                        HidD_GetFeature(read.0, feature.as_mut_ptr().cast(), feature.len() as u32)
                    };
                }
                let output = Handle::open(path, GENERIC_READ | GENERIC_WRITE)
                    .map(|h| (sony, h, bluetooth, usize::from(caps.OutputReportByteLength)));
                (Reader::Sony(sony), output)
            }
            None => {
                let mut values =
                    vec![HIDP_VALUE_CAPS::default(); usize::from(caps.NumberInputValueCaps)];
                let mut count = caps.NumberInputValueCaps;
                // SAFETY: room for `count` capabilities.
                let ok = unsafe {
                    HidP_GetValueCaps(HidP_Input, values.as_mut_ptr(), &mut count, preparsed.0)
                } == HIDP_STATUS_SUCCESS;
                values.truncate(if ok { usize::from(count) } else { 0 });
                let mut axes = Vec::new();
                let mut hat = None;
                for v in values.iter().filter(|v| v.UsagePage == 0x01) {
                    // SAFETY: the union's variant is the one `IsRange` names.
                    let (first, last) = unsafe {
                        if v.IsRange {
                            (v.Anonymous.Range.UsageMin, v.Anonymous.Range.UsageMax)
                        } else {
                            (v.Anonymous.NotRange.Usage, v.Anonymous.NotRange.Usage)
                        }
                    };
                    let (min, max) = range(v);
                    for usage in first..=last {
                        match usage {
                            u if (USAGE_X..USAGE_X + AXES as u16).contains(&u) => {
                                axes.push((usize::from(u - USAGE_X), min, max));
                            }
                            USAGE_HAT => hat = Some((min, max)),
                            _ => {}
                        }
                    }
                }
                let reader = Reader::Generic {
                    preparsed,
                    axes,
                    hat,
                    map,
                };
                (reader, None)
            }
        };
        tracing::info!(
            vendor = format_args!("{:04x}", attributes.VendorID),
            product = format_args!("{:04x}", attributes.ProductID),
            kind = match &reader {
                Reader::Sony(s) => format!("{s:?}"),
                Reader::Switch2 { .. } => "Switch 2 Pro".into(),
                Reader::Generic { .. } => "generic".into(),
            },
            "HID pad connected"
        );
        let pad = Pad {
            id,
            path: path.to_vec(),
            state: Mutex::new(None),
            connected: AtomicBool::new(true),
            output,
            rumble: Mutex::new((0, 0)),
        };
        Some((pad, reader, usize::from(caps.InputReportByteLength), read))
    }

    /// The Switch 2 pads' command interface (USB interface 1, on WinUSB).
    const SWITCH2_COMMANDS: GUID = GUID::from_u128(0x6F13725E_EF0E_4FD3_AE5F_B2DE989EC825);

    /// The Switch 2 Pro Controller's command interface, set up the way SDL
    /// does it: the pad sends no input until it is. Steam does the same
    /// while it runs and keeps the interface to itself; the pad then
    /// reports already and only the calibration is missing.
    struct Commands {
        usb: WINUSB_INTERFACE_HANDLE,
        out: u8,
        input: u8,
        _file: Handle,
    }

    // SAFETY: WinUSB handles may be used from any thread.
    unsafe impl Send for Commands {}

    impl Drop for Commands {
        fn drop(&mut self) {
            // SAFETY: our own interface handle, freed once, before its file.
            unsafe { WinUsb_Free(self.usb) };
        }
    }

    impl Commands {
        /// The first Switch 2 Pro Controller's interface, if no one else
        /// holds it.
        fn open() -> Option<Commands> {
            let path = interface_paths(&SWITCH2_COMMANDS).into_iter().find(|p| {
                String::from_utf16_lossy(p)
                    .to_ascii_lowercase()
                    .contains("vid_057e&pid_2069")
            })?;
            let Some(file) =
                Handle::open_with(&path, GENERIC_READ | GENERIC_WRITE, FILE_FLAG_OVERLAPPED)
            else {
                tracing::info!(
                    error = %std::io::Error::last_os_error(),
                    "Switch 2 pad: its command interface is in use (Steam?), read uncalibrated"
                );
                return None;
            };
            let mut usb = std::ptr::null_mut();
            // SAFETY: an open WinUSB device file.
            if unsafe { WinUsb_Initialize(file.0, &mut usb) } == 0 {
                return None;
            }
            let mut commands = Commands {
                usb,
                out: 0,
                input: 0,
                _file: file,
            };
            for index in 0..u8::MAX {
                let mut pipe = WINUSB_PIPE_INFORMATION::default();
                // SAFETY: an initialised interface; `pipe` is written.
                if unsafe { WinUsb_QueryPipe(usb, 0, index, &mut pipe) } == 0 {
                    break;
                }
                if pipe.PipeType == UsbdPipeTypeBulk {
                    if pipe.PipeId & 0x80 != 0 {
                        commands.input = pipe.PipeId;
                    } else {
                        commands.out = pipe.PipeId;
                    }
                }
            }
            if commands.input == 0 || commands.out == 0 {
                return None;
            }
            for (pipe, ms) in [(commands.out, 1000u32), (commands.input, 100)] {
                // SAFETY: as above; the value is read during the call.
                unsafe {
                    WinUsb_SetPipePolicy(
                        usb,
                        pipe,
                        PIPE_TRANSFER_TIMEOUT,
                        4,
                        (&ms as *const u32).cast(),
                    )
                };
            }
            Some(commands)
        }

        /// Sends a command and returns the reply, up to `length` bytes.
        fn send(&self, command: &[u8], length: usize) -> Vec<u8> {
            let mut written = 0;
            // SAFETY: an initialised interface; buffers outlive the calls.
            if unsafe {
                WinUsb_WritePipe(
                    self.usb,
                    self.out,
                    command.as_ptr(),
                    command.len() as u32,
                    &mut written,
                    std::ptr::null(),
                )
            } == 0
            {
                return Vec::new();
            }
            let mut reply = Vec::new();
            while reply.len() < length {
                let mut chunk = [0u8; 64];
                let mut got = 0;
                // SAFETY: as above.
                let ok = unsafe {
                    WinUsb_ReadPipe(
                        self.usb,
                        self.input,
                        chunk.as_mut_ptr(),
                        chunk.len() as u32,
                        &mut got,
                        std::ptr::null(),
                    )
                } != 0;
                if !ok {
                    break;
                }
                reply.extend_from_slice(&chunk[..got as usize]);
                if (got as usize) < chunk.len() {
                    break;
                }
            }
            reply
        }

        /// 0x40 bytes of the pad's flash from `address`.
        fn flash(&self, address: u32) -> Option<Vec<u8>> {
            let mut command = vec![0x02, 0x91, 0x00, 0x01, 0x00, 0x08, 0x00, 0x00, 0, 0, 0, 0];
            command.extend_from_slice(&address.to_le_bytes());
            self.send(&command, 0x50).get(0x10..0x50).map(<[u8]>::to_vec)
        }

        /// Reads the sticks' calibration, then starts the pad's 0x09
        /// reports.
        fn set_up(&self) -> Calibration {
            let mut cal = Calibration::default();
            let mut stick = |at: usize, block: Option<&[u8]>| {
                if let Some([x, y]) = block.and_then(stick_calibration) {
                    cal[at] = Some(x);
                    cal[at + 1] = Some(y);
                }
            };
            // Factory calibration, then the player's own if they made one.
            for (at, address) in [(0, 0x13080), (2, 0x130C0)] {
                let factory = self.flash(address);
                stick(at, factory.as_deref().and_then(|d| d.get(0x28..)));
            }
            for (at, address) in [(0, 0x1FC040), (2, 0x1FC080)] {
                let user = self.flash(address);
                let user = user.as_deref().filter(|d| d.starts_with(&[0xB2, 0xA1]));
                stick(at, user.and_then(|d| d.get(2..)));
            }
            let mut report_layout = [0u8; 28];
            report_layout[..9]
                .copy_from_slice(&[0x0A, 0x91, 0x00, 0x08, 0x00, 0x14, 0x00, 0x00, 0x01]);
            report_layout[9..17].fill(0xFF);
            report_layout[17..21].copy_from_slice(&[0x35, 0x00, 0x46, 0x00]);
            let sequence: [&[u8]; 10] = [
                &[0x07, 0x91, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00],
                &[0x0C, 0x91, 0x00, 0x02, 0x00, 0x04, 0x00, 0x00, 0x27, 0x00, 0x00, 0x00],
                &[0x11, 0x91, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00],
                &report_layout,
                &[0x0C, 0x91, 0x00, 0x04, 0x00, 0x04, 0x00, 0x00, 0x27, 0x00, 0x00, 0x00],
                &[0x01, 0x91, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x00],
                &[0x01, 0x91, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00],
                // Player 1's light.
                &[0x09, 0x91, 0x00, 0x07, 0x00, 0x08, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0, 0],
                // Reports as 0x09.
                &[0x03, 0x91, 0x00, 0x0A, 0x00, 0x04, 0x00, 0x00, 0x09, 0x00, 0x00, 0x00],
                // Start them.
                &[
                    0x03, 0x91, 0x00, 0x0D, 0x00, 0x08, 0x00, 0x00, 0x01, 0x00, 0xFF, 0xFF, 0xFF, 0xFF,
                    0xFF, 0xFF,
                ],
            ];
            for command in sequence {
                self.send(command, 0x40);
            }
            tracing::info!(calibrated = cal.iter().all(Option::is_some), "Switch 2 pad set up");
            cal
        }
    }

    /// The paths of the present device interfaces of a class, NUL-terminated.
    fn interface_paths(class: &GUID) -> Vec<Vec<u16>> {
        let mut size = 0;
        // SAFETY: the size is written; the list fits in `size` characters.
        unsafe {
            if CM_Get_Device_Interface_List_SizeW(
                &mut size,
                class,
                std::ptr::null(),
                CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
            ) != CR_SUCCESS
            {
                return Vec::new();
            }
            let mut list = vec![0u16; size as usize];
            if CM_Get_Device_Interface_ListW(
                class,
                std::ptr::null(),
                list.as_mut_ptr(),
                size,
                CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
            ) != CR_SUCCESS
            {
                return Vec::new();
            }
            list.split(|&c| c == 0)
                .filter(|p| !p.is_empty())
                .map(|p| p.iter().copied().chain([0]).collect())
                .collect()
        }
    }

    /// A value's logical range; a range declared unsigned in fewer bits than
    /// it is read in (a maximum that looks negative) is taken as unsigned.
    fn range(v: &HIDP_VALUE_CAPS) -> (i64, i64) {
        let (min, max) = (i64::from(v.LogicalMin), i64::from(v.LogicalMax));
        if max < min && v.BitSize > 0 && v.BitSize < 32 {
            let mask = (1i64 << v.BitSize) - 1;
            (min & mask, max & mask)
        } else {
            (min, max)
        }
    }

    /// Reads pad reports until the device goes away.
    fn listen(pad: &Pad, reader: &Reader, length: usize, read: &Handle) {
        let mut report = vec![0u8; length.max(1)];
        loop {
            let mut got = 0;
            // SAFETY: room for one report.
            let ok = unsafe {
                ReadFile(
                    read.0,
                    report.as_mut_ptr(),
                    report.len() as u32,
                    &mut got,
                    std::ptr::null_mut(),
                )
            } != 0;
            if !ok || got == 0 {
                break;
            }
            if let Some(state) = reader.read(&mut report[..got as usize]) {
                *super::super::lock(&pad.state) = Some(state);
            }
        }
        pad.connected.store(false, Ordering::Relaxed);
        tracing::info!("HID pad disconnected");
    }

    /// The device paths of every present HID interface, NUL-terminated.
    fn hid_paths() -> Vec<Vec<u16>> {
        let mut guid = GUID::from_u128(0);
        let mut paths = Vec::new();
        // SAFETY: SetupAPI enumeration with buffers we own; the list is
        // destroyed at the end.
        unsafe {
            HidD_GetHidGuid(&mut guid);
            let set = SetupDiGetClassDevsW(
                &guid,
                std::ptr::null(),
                std::ptr::null_mut(),
                DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
            );
            if set == INVALID_HANDLE_VALUE as isize {
                return paths;
            }
            for index in 0.. {
                let mut interface = SP_DEVICE_INTERFACE_DATA {
                    cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
                    ..Default::default()
                };
                if SetupDiEnumDeviceInterfaces(set, std::ptr::null(), &guid, index, &mut interface)
                    == 0
                {
                    break;
                }
                let mut size = 0;
                SetupDiGetDeviceInterfaceDetailW(
                    set,
                    &interface,
                    std::ptr::null_mut(),
                    0,
                    &mut size,
                    std::ptr::null_mut(),
                );
                if size == 0 {
                    continue;
                }
                // A u32-aligned buffer: cbSize, then the path.
                let mut buffer = vec![0u32; (size as usize).div_ceil(4)];
                let detail = buffer
                    .as_mut_ptr()
                    .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
                (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
                if SetupDiGetDeviceInterfaceDetailW(
                    set,
                    &interface,
                    detail,
                    size,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                ) == 0
                {
                    continue;
                }
                let start = std::ptr::addr_of!((*detail).DevicePath).cast::<u16>();
                let chars = (size as usize - 4) / 2;
                let path = std::slice::from_raw_parts(start, chars);
                let end = path.iter().position(|&c| c == 0).unwrap_or(path.len());
                paths.push(path[..end].iter().copied().chain([0]).collect());
            }
            SetupDiDestroyDeviceInfoList(set);
        }
        paths
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A DualSense USB report: sticks, triggers, counter, three button
    /// bytes, the rest zero.
    fn dualsense_usb(sticks: [u8; 4], triggers: [u8; 2], buttons: [u8; 3]) -> Vec<u8> {
        let mut r = vec![0u8; 64];
        r[0] = 0x01;
        r[1..5].copy_from_slice(&sticks);
        r[5..7].copy_from_slice(&triggers);
        r[8..11].copy_from_slice(&buttons);
        r
    }

    #[test]
    fn a_dualsense_on_usb_reads_as_a_console_pad() {
        // Cross, R1, Options, D-pad down-left; left stick fully up and
        // left, right stick at rest, L2 full.
        let r = dualsense_usb([0, 0, 128, 128], [255, 0], [0x20 | 5, 0x02 | 0x20, 0]);
        let pad = sony_report(Sony::DualSense, &r).unwrap();
        assert_eq!(pad.buttons, A | RIGHT_SHOULDER | MENU | DOWN | LEFT);
        assert_eq!((pad.left_x, pad.left_y), (-1.0, 1.0));
        assert_eq!((pad.right_x, pad.right_y), (0.0, 0.0));
        assert_eq!((pad.left_trigger, pad.right_trigger), (1.0, 0.0));
    }

    #[test]
    fn a_dualsense_on_bluetooth_reads_in_both_report_forms() {
        // The full report: the USB layout one byte later.
        let usb = dualsense_usb([255, 255, 0, 255], [0, 128], [0x80 | 8, 0x40, 0x02]);
        let mut full = vec![0x31, 0x00];
        full.extend(&usb[1..]);
        full.resize(78, 0);
        let pad = sony_report(Sony::DualSense, &full).unwrap();
        assert_eq!(pad, sony_report(Sony::DualSense, &usb).unwrap());
        assert_eq!(pad.buttons, Y | LEFT_STICK | VIEW);
        assert_eq!(
            (pad.left_x, pad.left_y, pad.right_x, pad.right_y),
            (1.0, -1.0, -1.0, -1.0)
        );
        // The short report before the full ones are asked for: buttons
        // before triggers.
        let short = [0x01, 128, 128, 128, 128, 0x10, 0x01, 0, 0, 255];
        let pad = sony_report(Sony::DualSense, &short).unwrap();
        assert_eq!(pad.buttons, X | UP | LEFT_SHOULDER);
        assert_eq!(pad.right_trigger, 1.0);
    }

    #[test]
    fn a_dualshock_4_reads_on_usb_and_bluetooth() {
        let mut usb = vec![0u8; 64];
        usb[0] = 0x01;
        usb[1..5].copy_from_slice(&[128, 128, 255, 0]);
        usb[5] = 0x40 | 2; // circle, D-pad right
        usb[6] = 0x10 | 0x80; // share, R3
        usb[8] = 0;
        usb[9] = 255;
        let pad = sony_report(Sony::DualShock4, &usb).unwrap();
        assert_eq!(pad.buttons, B | RIGHT | VIEW | RIGHT_STICK);
        assert_eq!(
            (pad.right_x, pad.right_y, pad.right_trigger),
            (1.0, 1.0, 1.0)
        );
        let mut bt = vec![0x11, 0xC0, 0x00];
        bt.extend(&usb[1..]);
        bt.resize(78, 0);
        assert_eq!(sony_report(Sony::DualShock4, &bt), Some(pad));
        // Reports that are not input state.
        assert_eq!(sony_report(Sony::DualShock4, &[0x05, 0, 0]), None);
        assert_eq!(sony_report(Sony::DualSense, &[]), None);
    }

    #[test]
    fn the_crc_is_ieee_crc_32() {
        assert_eq!(crc32(&[b"1234", b"56789"]), 0xCBF4_3926);
    }

    #[test]
    fn rumble_reports_put_the_motors_where_sony_reads_them() {
        let r = sony_rumble(Sony::DualSense, false, 48, 0xFF00, 0x8000);
        assert_eq!(r.len(), 48);
        assert_eq!(&r[..5], &[0x02, 0x03, 0x00, 0x80, 0xFF]);
        let r = sony_rumble(Sony::DualShock4, false, 32, 0x1000, 0x2000);
        assert_eq!(r.len(), 32);
        assert_eq!(&r[..6], &[0x05, 0x01, 0x00, 0x00, 0x20, 0x10]);
        for pad in [Sony::DualSense, Sony::DualShock4] {
            let r = sony_rumble(pad, true, 78, 0xFFFF, 0);
            assert_eq!(r.len(), 78);
            let crc = crc32(&[&[0xA2], &r[..74]]);
            assert_eq!(&r[74..], &crc.to_le_bytes());
        }
        let r = sony_rumble(Sony::DualSense, true, 78, 0xFFFF, 0);
        assert_eq!(&r[..7], &[0x31, 0x00, 0x10, 0x03, 0x00, 0x00, 0xFF]);
        let r = sony_rumble(Sony::DualShock4, true, 78, 0xFFFF, 0);
        assert_eq!(&r[..8], &[0x11, 0xC0, 0x00, 0x01, 0x00, 0x00, 0x00, 0xFF]);
    }

    #[test]
    fn sony_pads_are_known_by_vendor_and_product() {
        assert_eq!(Sony::of(0x054C, 0x0CE6), Some(Sony::DualSense));
        assert_eq!(Sony::of(0x054C, 0x0DF2), Some(Sony::DualSense));
        assert_eq!(Sony::of(0x054C, 0x09CC), Some(Sony::DualShock4));
        assert_eq!(Sony::of(0x045E, 0x0B12), None);
    }

    #[test]
    fn a_generic_pad_maps_by_the_directinput_layout() {
        // Sticks on X/Y and Z/Rz; button 2 (south), 10 (start), hat left.
        let g = Generic {
            axes: [Some(-1.0), Some(1.0), Some(0.5), None, None, Some(-0.5)],
            hat: Some(6),
            buttons: vec![2, 10, 7],
        };
        let pad = generic(&g, &ButtonMap::DIRECTINPUT);
        assert_eq!(pad.buttons, A | MENU | LEFT);
        assert_eq!((pad.left_x, pad.left_y), (-1.0, -1.0));
        assert_eq!((pad.right_x, pad.right_y), (0.5, 0.5));
        assert_eq!((pad.left_trigger, pad.right_trigger), (1.0, 0.0));
    }

    #[test]
    fn a_generic_pad_with_six_axes_has_analogue_triggers() {
        // Z/Rz the right stick, Rx/Ry the triggers (-1 released).
        let g = Generic {
            axes: [
                Some(0.0),
                Some(0.0),
                Some(0.0),
                Some(1.0),
                Some(-1.0),
                Some(0.0),
            ],
            hat: None,
            buttons: vec![],
        };
        let pad = generic(&g, &ButtonMap::DIRECTINPUT);
        assert_eq!((pad.left_trigger, pad.right_trigger), (1.0, 0.0));
        assert_eq!(pad.buttons, 0);
        // Without Z, Rx/Ry are the right stick.
        let g = Generic {
            axes: [None, None, None, Some(1.0), Some(-1.0), None],
            ..g
        };
        let pad = generic(&g, &ButtonMap::DIRECTINPUT);
        assert_eq!((pad.right_x, pad.right_y), (1.0, 1.0));
        assert_eq!((pad.left_trigger, pad.right_trigger), (0.0, 0.0));
    }

    #[test]
    fn the_button_map_can_be_named() {
        assert_eq!(
            ButtonMap::parse(Some("1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12")),
            ButtonMap([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12])
        );
        assert_eq!(ButtonMap::parse(Some("1,2,3")), ButtonMap::DIRECTINPUT);
        assert_eq!(ButtonMap::parse(Some("a,b")), ButtonMap::DIRECTINPUT);
        assert_eq!(ButtonMap::parse(None), ButtonMap::DIRECTINPUT);
    }

    #[test]
    fn a_switch_2_pro_reads_by_button_position() {
        // A report Steam's set-up gives: nothing held, sticks near rest.
        let mut steam = vec![0u8; 64];
        steam[..16].copy_from_slice(&[
            0x05, 0x19, 0xE4, 0x13, 0, 0, 0, 0, 0, 0, 0, 0x36, 0x48, 0x86, 0x98, 0x68,
        ]);
        let none = Calibration::default();
        let pad = switch2_report(&steam, &none).unwrap();
        assert_eq!(pad.buttons, 0);
        assert!((pad.left_x - 54.0 / 2048.0).abs() < 1e-4);
        assert!((pad.left_y - 100.0 / 2048.0).abs() < 1e-4);
        // Nintendo's B (bottom), Plus, D-pad up.
        steam[5] = 0x04;
        steam[6] = 0x02;
        steam[7] = 0x02;
        let pad = switch2_report(&steam, &none).unwrap();
        assert_eq!(pad.buttons, A | MENU | UP);
        assert_eq!((pad.left_trigger, pad.right_trigger), (0.0, 0.0));
        steam[7] = 0x80;
        steam[5] = 0x80;
        let pad = switch2_report(&steam, &none).unwrap();
        assert_eq!((pad.left_trigger, pad.right_trigger), (1.0, 1.0));

        // Our own 0x09 reports: Nintendo's A (right), Y (left), Minus, L,
        // ZR; left stick fully up against its calibration.
        let mut own = vec![0u8; 64];
        own[0] = 0x09;
        own[3] = 0x02 | 0x20;
        own[4] = 0x04 | 0x40 | 0x10;
        own[6..12].copy_from_slice(&[0x00, 0x08, 0xC0, 0x00, 0x08, 0x80]);
        let travel = Travel {
            neutral: 2048,
            above: 1024,
            below: 1024,
        };
        let pad = switch2_report(&own, &[Some(travel); 4]).unwrap();
        assert_eq!(pad.buttons, B | LEFT | VIEW | LEFT_SHOULDER);
        assert_eq!((pad.left_trigger, pad.right_trigger), (0.0, 1.0));
        assert_eq!((pad.left_x, pad.left_y), (0.0, 1.0));
        assert_eq!((pad.right_x, pad.right_y), (0.0, 0.0));
        assert_eq!(switch2_report(&own[..11], &none), None);
        assert_eq!(switch2_report(&[0x30, 0, 0], &none), None);
    }

    #[test]
    fn switch_2_calibration_is_packed_in_twelve_bits() {
        let block = [0x00, 0x88, 0x80, 0x00, 0x05, 0x50, 0x10, 0x05, 0x52];
        let [x, y] = stick_calibration(&block).unwrap();
        let travel = |neutral, above, below| Travel {
            neutral,
            above,
            below,
        };
        assert_eq!(x, travel(0x800, 0x500, 0x510));
        assert_eq!(y, travel(0x808, 0x500, 0x520));
        assert_eq!(stick_calibration(&[0; 8]), None);
        assert!(is_switch2_pro(0x057E, 0x2069));
        assert!(!is_switch2_pro(0x057E, 0x2009));
    }

    /// With a pad connected: rumbles each HID pad for a second, then prints
    /// every change of state for `VICTORY_PAD_SECONDS` (default 20).
    #[cfg(windows)]
    #[test]
    #[ignore = "needs a pad: cargo test -p victory-hle connected_hid_pads -- --ignored --nocapture"]
    fn connected_hid_pads() {
        use std::time::{Duration, Instant};
        let pads = Pads::default();
        let found = Instant::now();
        while pads.connected().is_empty() && found.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(100));
        }
        let ids: Vec<u64> = pads.connected().iter().map(|&(id, _)| id).collect();
        eprintln!("HID pads: {ids:?}");
        for &id in &ids {
            pads.rumble(id, 0xFFFF, 0x8000);
        }
        std::thread::sleep(Duration::from_secs(1));
        for &id in &ids {
            pads.rumble(id, 0, 0);
        }
        let seconds = std::env::var("VICTORY_PAD_SECONDS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(20);
        let end = Instant::now() + Duration::from_secs(seconds);
        let mut last = Vec::new();
        while Instant::now() < end {
            let now = pads.connected();
            if now != last {
                for (id, pad) in &now {
                    eprintln!(
                        "pad {id}: buttons {:#06x} lt {:.2} rt {:.2} left ({:.2}, {:.2}) right ({:.2}, {:.2})",
                        pad.buttons,
                        pad.left_trigger,
                        pad.right_trigger,
                        pad.left_x,
                        pad.left_y,
                        pad.right_x,
                        pad.right_y
                    );
                }
                last = now;
            }
            std::thread::sleep(Duration::from_millis(8));
        }
    }

    #[test]
    fn values_normalise_over_their_range() {
        assert_eq!(normalise(0, 0, 255), -1.0);
        assert_eq!(normalise(255, 0, 255), 1.0);
        assert!(normalise(0, -32768, 32767).abs() < 0.001);
        assert_eq!(normalise(5, 5, 5), 0.0);
    }
}
