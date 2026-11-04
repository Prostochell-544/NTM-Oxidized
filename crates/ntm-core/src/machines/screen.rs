//! A machine's screen as data: the panel, bars, icons, tooltips and buttons NTM's GUI classes draw
//! by hand. [`blits`], [`tooltip`] and [`click`] work it out from the menu data; a branch only puts
//! the pieces on screen and writes the labels.

use crate::battery;

/// A number the screen shows.
#[derive(Clone, Copy, Debug)]
pub enum Value {
    /// One menu data slot (16 bits on the network).
    Data(usize),
    /// A long in four data slots from this one (`battery::to_data`).
    Long(usize),
    Const(i64),
}

impl Value {
    pub fn get(self, data: &[i32]) -> i64 {
        match self {
            Value::Data(i) => data.get(i).copied().map_or(0, i64::from),
            Value::Long(i) => data.get(i..i + 4).map_or(0, battery::from_data),
            Value::Const(n) => n,
        }
    }
}

/// Which way a bar fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fill {
    Up,
    Right,
    Down,
}

/// A piece of the texture, at `src`, drawn at `dst` (both from the top left), `size` big.
#[derive(Clone, Copy, Debug)]
pub enum Part {
    Fixed { src: [f32; 2], dst: [f32; 2], size: [f32; 2] },
    /// `value / max` of the full `size`, filling `fill`-wards, `extra` pixels longer (as some GUIs
    /// draw their arrows one wider).
    Bar { value: Value, max: Value, fill: Fill, extra: f32, src: [f32; 2], dst: [f32; 2], size: [f32; 2] },
    /// Shown while `value` is at least `min`.
    When { value: Value, min: i64, src: [f32; 2], dst: [f32; 2], size: [f32; 2] },
    /// One of a row of icons: the one at `src + (value - base) * step`, none when `value` is `hide`.
    Pick { value: Value, base: i64, step: [f32; 2], hide: Option<i64>, src: [f32; 2], dst: [f32; 2], size: [f32; 2] },
}

/// What a tooltip says.
#[derive(Clone, Copy, Debug)]
pub enum Text {
    /// `power/max HE`.
    Energy { value: Value, max: Value },
    /// `value * num / den` and a suffix: `75%`, `3 operations left`.
    Scaled { value: Value, num: i64, den: i64, suffix: &'static str },
    /// Fixed lines, only while one of `values` is one of `when` if that is given.
    Lines { lines: &'static [&'static str], when: Option<(&'static [Value], &'static [i64])> },
    /// The battery socket's charge and change per second, while it holds a battery.
    Charge { charge: Value, max: Value, delta: Value },
    /// The battery socket's priority, explained.
    Priority { value: Value },
}

/// A tooltip over `area` (`[x, y, width, height]`).
#[derive(Clone, Copy, Debug)]
pub struct Tip {
    pub area: [f32; 4],
    pub text: Text,
}

/// A button: a click in `area` presses menu button `id` on the server.
#[derive(Clone, Copy, Debug)]
pub struct Button {
    pub area: [f32; 4],
    pub id: i32,
}

/// A label's place: centred on `x`, at `y`, in `color`.
#[derive(Clone, Copy, Debug)]
pub struct Label {
    pub x: f32,
    pub y: f32,
    pub color: u32,
}

pub const DARK: u32 = 0x404040;

#[derive(Clone, Copy, Debug)]
pub struct Screen {
    /// The GUI texture, `ntm:gui/...`.
    pub texture: &'static str,
    pub size: [f32; 2],
    /// Where each container slot is drawn.
    pub slots: &'static [Option<[f32; 2]>],
    /// The player inventory's top left.
    pub inventory: [f32; 2],
    /// The block's name, centred.
    pub title: Label,
    /// "Inventory", from its left.
    pub inventory_label: [f32; 2],
    pub parts: &'static [Part],
    pub tips: &'static [Tip],
    pub buttons: &'static [Button],
}

/// One piece of the texture onto the screen: `[u, v, w, h]` to `[x, y, w, h]` from its top left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blit {
    pub src: [f32; 4],
    pub dst: [f32; 4],
}

fn blit(src: [f32; 2], dst: [f32; 2], size: [f32; 2]) -> Blit {
    Blit { src: [src[0], src[1], size[0], size[1]], dst: [dst[0], dst[1], size[0], size[1]] }
}

/// The pieces to draw for this menu data, the panel first.
pub fn blits(screen: &Screen, data: &[i32]) -> Vec<Blit> {
    let mut out = vec![blit([0.0, 0.0], [0.0, 0.0], screen.size)];
    for part in screen.parts {
        match *part {
            Part::Fixed { src, dst, size } => out.push(blit(src, dst, size)),
            Part::Bar { value, max, fill, extra, src, dst, size } => {
                let (mut value, mut max) = (value.get(data), max.get(data));
                if max <= 0 {
                    continue;
                }
                // As the socket's GUI does, so the product can't overflow.
                if value > i64::MAX / 100 {
                    value /= 100;
                    max /= 100;
                }
                let full = if fill == Fill::Right { size[0] } else { size[1] };
                let length = (value * full as i64 / max.max(1)) as f32 + extra;
                if length <= 0.0 {
                    continue;
                }
                out.push(match fill {
                    Fill::Up => blit(
                        [src[0], src[1] + size[1] - length],
                        [dst[0], dst[1] + size[1] - length],
                        [size[0], length],
                    ),
                    Fill::Down => blit(src, dst, [size[0], length]),
                    Fill::Right => blit(src, dst, [length, size[1]]),
                });
            }
            Part::When { value, min, src, dst, size } => {
                if value.get(data) >= min {
                    out.push(blit(src, dst, size));
                }
            }
            Part::Pick { value, base, step, hide, src, dst, size } => {
                let value = value.get(data);
                if Some(value) != hide {
                    let n = (value - base) as f32;
                    out.push(blit([src[0] + n * step[0], src[1] + n * step[1]], dst, size));
                }
            }
        }
    }
    out
}

fn inside(area: [f32; 4], at: [f32; 2]) -> bool {
    (area[0]..area[0] + area[2]).contains(&at[0]) && (area[1]..area[1] + area[3]).contains(&at[1])
}

/// The tooltip lines at `at`, from the screen's top left.
pub fn tooltip(screen: &Screen, data: &[i32], at: [f32; 2]) -> Vec<String> {
    let Some(tip) = screen.tips.iter().find(|tip| inside(tip.area, at)) else {
        return Vec::new();
    };
    match tip.text {
        Text::Energy { value, max } => vec![format!("{}/{} HE", value.get(data), max.get(data))],
        Text::Scaled { value, num, den, suffix } => {
            vec![format!("{}{suffix}", value.get(data) * num / den.max(1))]
        }
        Text::Lines { lines, when } => {
            let shown = when.is_none_or(|(values, any)| values.iter().any(|v| any.contains(&v.get(data))));
            if shown { lines.iter().map(|line| line.to_string()).collect() } else { Vec::new() }
        }
        Text::Charge { charge, max, delta } => {
            let (charge, max, delta) = (charge.get(data), max.get(data), delta.get(data));
            if max <= 0 {
                return Vec::new();
            }
            let sign = if delta < 0 { "-" } else { "+" };
            vec![
                format!("{}/{}HE", battery::short_number(charge), battery::short_number(max)),
                format!("{sign}{}HE/s", battery::short_number(delta.abs())),
            ]
        }
        Text::Priority { value } => {
            let priority = crate::energy::Priority::from_ordinal(value.get(data) as i32);
            battery::priority_lines(priority).iter().map(|line| line.to_string()).collect()
        }
    }
}

/// The menu button a click at `at` presses.
pub fn click(screen: &Screen, at: [f32; 2]) -> Option<i32> {
    screen.buttons.iter().find(|button| inside(button.area, at)).map(|button| button.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Screen = Screen {
        texture: "ntm:gui/test",
        size: [176.0, 166.0],
        slots: &[],
        inventory: [8.0, 84.0],
        title: Label { x: 88.0, y: 6.0, color: DARK },
        inventory_label: [8.0, 72.0],
        parts: &[
            Part::Bar { value: Value::Data(0), max: Value::Const(100), fill: Fill::Up, extra: 0.0, src: [176.0, 0.0], dst: [10.0, 10.0], size: [16.0, 50.0] },
            Part::Bar { value: Value::Data(1), max: Value::Const(40), fill: Fill::Right, extra: 1.0, src: [176.0, 60.0], dst: [80.0, 30.0], size: [24.0, 17.0] },
            Part::Pick { value: Value::Data(2), base: 1, step: [0.0, 18.0], hide: Some(0), src: [200.0, 0.0], dst: [40.0, 40.0], size: [18.0, 18.0] },
        ],
        tips: &[Tip { area: [10.0, 10.0, 16.0, 50.0], text: Text::Energy { value: Value::Data(0), max: Value::Const(100) } }],
        buttons: &[Button { area: [100.0, 10.0, 18.0, 18.0], id: 2 }],
    };

    #[test]
    fn bars_fill_from_their_end_and_icons_follow_the_value() {
        let b = blits(&SCREEN, &[50, 20, 3]);
        assert_eq!(b[0].dst, [0.0, 0.0, 176.0, 166.0]);
        assert_eq!(b[1], Blit { src: [176.0, 25.0, 16.0, 25.0], dst: [10.0, 35.0, 16.0, 25.0] });
        assert_eq!(b[2].src, [176.0, 60.0, 13.0, 17.0]);
        assert_eq!(b[3].src, [200.0, 36.0, 18.0, 18.0]);
        assert_eq!(blits(&SCREEN, &[0, 0, 0]).len(), 2, "an empty bar and a hidden icon draw nothing");
    }

    #[test]
    fn tooltips_and_buttons_answer_by_area() {
        assert_eq!(tooltip(&SCREEN, &[42, 0, 0], [12.0, 20.0]), vec!["42/100 HE"]);
        assert!(tooltip(&SCREEN, &[42, 0, 0], [90.0, 90.0]).is_empty());
        assert_eq!(click(&SCREEN, [105.0, 15.0]), Some(2));
        assert_eq!(click(&SCREEN, [5.0, 5.0]), None);
    }

    #[test]
    fn longs_in_four_slots_read_back() {
        let mut data = vec![7];
        data.extend(battery::to_data(72_000_000_000));
        assert_eq!(Value::Long(1).get(&data), 72_000_000_000);
        assert_eq!(Value::Data(0).get(&data), 7);
        assert_eq!(Value::Data(9).get(&data), 0);
    }
}
