//! Four concrete panel slots. Geometry is in DIP and never owns editor text.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelId {
    Explorer,
    Calendar,
    Outline,
    Git,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    LeftTop,
    LeftBottom,
    RightTop,
    RightBottom,
}
const IDS: [PanelId; 4] = [
    PanelId::Explorer,
    PanelId::Calendar,
    PanelId::Outline,
    PanelId::Git,
];
const SLOTS: [Slot; 4] = [
    Slot::LeftTop,
    Slot::LeftBottom,
    Slot::RightTop,
    Slot::RightBottom,
];
const NAMES: [&str; 4] = ["explorer", "calendar", "outline", "git"];
const SLOT_NAMES: [&str; 4] = ["left_top", "left_bottom", "right_top", "right_bottom"];

#[derive(Clone, Debug, PartialEq)]
pub struct PanelState {
    pub id: PanelId,
    pub slot: Slot,
    pub width: f32,
    pub height: f32,
    pub collapsed: bool,
    pub hidden: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct PanelLayout {
    pub panels: [PanelState; 4],
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
impl Rect {
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
}
pub struct Geometry {
    pub editor: Rect,
    pub panels: [(PanelId, Rect); 4],
}
impl Default for PanelLayout {
    fn default() -> Self {
        Self {
            panels: std::array::from_fn(|i| PanelState {
                id: IDS[i],
                slot: SLOTS[i],
                width: if i < 2 { 240.0 } else { 260.0 },
                height: 240.0,
                collapsed: false,
                hidden: false,
            }),
        }
    }
}
impl PanelLayout {
    pub fn get(&self, id: PanelId) -> &PanelState {
        &self.panels[id as usize]
    }
    pub fn get_mut(&mut self, id: PanelId) -> &mut PanelState {
        &mut self.panels[id as usize]
    }
    pub fn move_to(&mut self, id: PanelId, slot: Slot) {
        let old = self.get(id).slot;
        if let Some(other) = self.panels.iter_mut().find(|p| p.slot == slot) {
            other.slot = old;
        }
        self.get_mut(id).slot = slot;
    }
    pub fn resize(&mut self, id: PanelId, width: f32, height: f32) -> Result<(), &'static str> {
        if !valid_dimension(width) || !valid_dimension(height) {
            return Err("panel dimensions must be 1..4096 DIP");
        }
        let p = self.get_mut(id);
        p.width = width;
        p.height = height;
        Ok(())
    }
    /// Reserve the central editor first; side panes share only the remaining width.
    /// At very narrow widths panes collapse to zero, remaining reachable by Activity.
    pub fn geometry(&self, width: f32, height: f32, top: f32, bottom: f32) -> Geometry {
        let width = finite_nonnegative(width);
        let height = finite_nonnegative(height);
        let activity = 44.0_f32.min(width);
        let top = finite_nonnegative(top).min(height);
        let body = (height - top - finite_nonnegative(bottom)).max(0.0);
        let available = (width - activity).max(0.0);
        let sides = (available - 320.0).max(0.0);
        let side_width = |left: bool| {
            self.panels
                .iter()
                .filter(|p| !p.hidden && is_left(p.slot) == left)
                .map(|p| p.width)
                .fold(0.0, f32::max)
        };
        let (mut left, mut right) = (side_width(true), side_width(false));
        if left + right > sides {
            let ratio = sides / (left + right);
            left *= ratio;
            right *= ratio;
        }
        let panels = std::array::from_fn(|i| {
            let p = &self.panels[i];
            let same_side: Vec<_> = self
                .panels
                .iter()
                .filter(|q| !q.hidden && is_left(q.slot) == is_left(p.slot))
                .collect();
            let lower = matches!(p.slot, Slot::LeftBottom | Slot::RightBottom);
            let upper = same_side
                .iter()
                .find(|q| matches!(q.slot, Slot::LeftTop | Slot::RightTop));
            let lower_panel = same_side
                .iter()
                .find(|q| matches!(q.slot, Slot::LeftBottom | Slot::RightBottom));
            let split = match (upper, lower_panel) {
                (Some(u), Some(_)) if u.collapsed => 30.0_f32.min(body),
                (Some(_), Some(l)) if l.collapsed => (body - 30.0).max(0.0),
                (Some(u), Some(l)) => body * u.height / (u.height + l.height),
                (Some(_), None) => body,
                _ => 0.0,
            };
            let w = if is_left(p.slot) { left } else { right };
            let rect = if p.hidden {
                Rect::default()
            } else {
                Rect {
                    x: if is_left(p.slot) {
                        activity
                    } else {
                        width - right
                    },
                    y: top + if lower { split } else { 0.0 },
                    width: w,
                    height: {
                        let allocated = if lower { body - split } else { split };
                        if p.collapsed {
                            allocated.min(30.0)
                        } else {
                            allocated
                        }
                    },
                }
            };
            (p.id, rect)
        });
        Geometry {
            editor: Rect {
                x: activity + left,
                y: top,
                width: (available - left - right).max(0.0),
                height: body,
            },
            panels,
        }
    }
    /// Legacy schema 1 is retained: all four complete records, unique id and slot.
    pub fn parse(text: &str) -> Result<Self, &'static str> {
        let mut version = false;
        let mut records: Vec<std::collections::BTreeMap<&str, &str>> = Vec::new();
        for line in text.trim_start_matches('\u{feff}').lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line == "[[panel]]" {
                records.push(Default::default());
                continue;
            }
            let (key, value) = line.split_once('=').ok_or("invalid panel line")?;
            let (key, value) = (key.trim(), value.trim());
            if records.is_empty() {
                if key != "schema_version" || value != "1" || version {
                    return Err("unsupported panel schema");
                }
                version = true;
            } else if records.last_mut().unwrap().insert(key, value).is_some() {
                return Err("duplicate panel field");
            }
        }
        if !version || records.len() != 4 {
            return Err("all four panel records required");
        }
        let mut result = Self::default();
        let mut ids = [false; 4];
        let mut slots = [false; 4];
        for record in records {
            if record.len() != 6 {
                return Err("incomplete or unknown panel field");
            }
            let get = |key| record.get(key).copied().ok_or("missing panel field");
            let id = NAMES
                .iter()
                .position(|s| get("id").ok().and_then(unquote) == Some(*s))
                .ok_or("unknown panel id")?;
            let slot = SLOT_NAMES
                .iter()
                .position(|s| get("slot").ok().and_then(unquote) == Some(*s))
                .ok_or("unknown panel slot")?;
            if ids[id] || slots[slot] {
                return Err("duplicate panel id or slot");
            }
            ids[id] = true;
            slots[slot] = true;
            let width = get("width")?.parse().map_err(|_| "invalid panel width")?;
            let height = get("height")?.parse().map_err(|_| "invalid panel height")?;
            result.resize(IDS[id], width, height)?;
            let p = result.get_mut(IDS[id]);
            p.slot = SLOTS[slot];
            p.collapsed = get("collapsed")?.parse().map_err(|_| "invalid collapsed")?;
            p.hidden = get("hidden")?.parse().map_err(|_| "invalid hidden")?;
        }
        Ok(result)
    }
    pub fn serialize(&self) -> String {
        let mut text = String::from("schema_version = 1\n");
        for p in &self.panels {
            text.push_str(&format!("\n[[panel]]\nid = \"{}\"\nslot = \"{}\"\nwidth = {}\nheight = {}\ncollapsed = {}\nhidden = {}\n", NAMES[p.id as usize], SLOT_NAMES[p.slot as usize], p.width, p.height, p.collapsed, p.hidden));
        }
        text
    }
}
fn unquote(s: &str) -> Option<&str> {
    s.strip_prefix('"')?.strip_suffix('"')
}
fn is_left(slot: Slot) -> bool {
    matches!(slot, Slot::LeftTop | Slot::LeftBottom)
}
fn valid_dimension(n: f32) -> bool {
    n.is_finite() && (1.0..=4096.0).contains(&n)
}
fn finite_nonnegative(n: f32) -> f32 {
    if n.is_finite() { n.max(0.0) } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_roundtrip_rejects_duplicates_unknown_schema_and_nan() {
        let layout = PanelLayout::default();
        let text = layout.serialize();
        assert_eq!(PanelLayout::parse(&text).unwrap(), layout);
        assert!(PanelLayout::parse(&text.replacen("left_bottom", "left_top", 1)).is_err());
        assert!(
            PanelLayout::parse(&text.replace("schema_version = 1", "schema_version = 2")).is_err()
        );
        assert!(PanelLayout::parse(&text.replacen("width = 240", "width = NaN", 1)).is_err());
        assert!(PanelLayout::parse(&text.replacen("hidden = false", "", 1)).is_err());
    }
    #[test]
    fn narrow_geometry_reserves_editor_and_keeps_rects_in_window() {
        let mut layout = PanelLayout::default();
        for w in [0.0, 20.0, 250.0, 600.0, 1440.0] {
            let g = layout.geometry(w, 900.0, 76.0, 24.0);
            assert!(g.editor.width >= (w - 44.0).clamp(0.0, 320.0) - 0.01);
            for (_, r) in g.panels {
                assert!(r.x >= 0.0 && r.x + r.width <= w + 0.01 && r.height >= 0.0);
            }
        }
        layout.move_to(PanelId::Calendar, Slot::RightTop);
        assert_eq!(layout.get(PanelId::Outline).slot, Slot::LeftBottom);
        layout.get_mut(PanelId::Calendar).collapsed = true;
        let g = layout.geometry(1440.0, 900.0, 76.0, 24.0);
        assert_eq!(g.panels[1].1.height, 30.0);
        layout.get_mut(PanelId::Calendar).hidden = true;
        assert_eq!(
            layout.geometry(1440.0, 900.0, 76.0, 24.0).panels[1].1,
            Rect::default()
        );
        let mut collapsed = PanelLayout::default();
        collapsed.get_mut(PanelId::Explorer).collapsed = true;
        collapsed.get_mut(PanelId::Calendar).collapsed = true;
        let geometry = collapsed.geometry(1440.0, 900.0, 76.0, 24.0);
        assert_eq!(geometry.panels[0].1.height, 30.0);
        assert_eq!(geometry.panels[1].1.height, 30.0);
        collapsed.get_mut(PanelId::Explorer).hidden = true;
        assert_eq!(
            collapsed.geometry(1440.0, 900.0, 76.0, 24.0).panels[1]
                .1
                .height,
            30.0
        );
    }
}
