//! Canonical panel groups. Presentation geometry never owns editor text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelId {
    Explorer,
    Calendar,
    Outline,
    Git,
    Search,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}
impl Side {
    pub fn index(self) -> usize {
        usize::from(self == Self::Right)
    }
}
/// Legacy command positions, accepted as explicit move/join requests only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    LeftTop,
    LeftBottom,
    RightTop,
    RightBottom,
}
pub const IDS: [PanelId; 5] = [
    PanelId::Explorer,
    PanelId::Calendar,
    PanelId::Outline,
    PanelId::Git,
    PanelId::Search,
];
const NAMES: [&str; 5] = ["explorer", "calendar", "outline", "git", "search"];
const SLOT_NAMES: [&str; 4] = ["left_top", "left_bottom", "right_top", "right_bottom"];
#[derive(Clone, Debug, PartialEq)]
pub struct PanelState {
    pub id: PanelId,
    pub height: f32,
    pub collapsed: bool,
    pub hidden: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct PanelGroup {
    pub representative: PanelId,
    pub side: Side,
    pub views: Vec<PanelId>,
    pub width: f32,
}
#[derive(Clone, Debug, PartialEq)]
pub struct PanelLayout {
    pub panels: [PanelState; 5],
    pub groups: Vec<PanelGroup>,
    pub active: [Option<PanelId>; 2],
    pub left_visible: bool,
    pub right_visible: bool,
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
    pub panels: [(PanelId, Rect); 5],
}
impl Default for PanelLayout {
    fn default() -> Self {
        Self {
            panels: std::array::from_fn(|i| PanelState {
                id: IDS[i],
                height: 240.,
                collapsed: false,
                hidden: false,
            }),
            groups: [
                PanelId::Explorer,
                PanelId::Search,
                PanelId::Calendar,
                PanelId::Outline,
                PanelId::Git,
            ]
            .into_iter()
            .map(|id| PanelGroup {
                representative: id,
                side: Side::Left,
                views: vec![id],
                width: 240.,
            })
            .collect(),
            active: [Some(PanelId::Explorer), None],
            left_visible: true,
            right_visible: true,
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
    pub fn group(&self, id: PanelId) -> &PanelGroup {
        self.groups
            .iter()
            .find(|g| g.views.contains(&id))
            .expect("every view belongs to one group")
    }
    pub fn side(&self, id: PanelId) -> Side {
        self.group(id).side
    }
    pub fn region_visible(&self, id: PanelId) -> bool {
        if self.side(id) == Side::Left {
            self.left_visible
        } else {
            self.right_visible
        }
    }
    pub fn selected(&self, id: PanelId) -> bool {
        let g = self.group(id);
        self.active[g.side.index()] == Some(g.representative)
    }
    pub fn displayed(&self, id: PanelId) -> bool {
        self.selected(id) && self.region_visible(id) && !self.get(id).hidden
    }
    pub fn show_region_for(&mut self, id: PanelId) {
        if self.side(id) == Side::Left {
            self.left_visible = true;
        } else {
            self.right_visible = true;
        }
    }
    pub fn select(&mut self, id: PanelId) {
        let g = self.group(id);
        self.active[g.side.index()] = Some(g.representative);
    }
    pub fn set_width(&mut self, id: PanelId, width: f32) {
        self.groups
            .iter_mut()
            .find(|g| g.views.contains(&id))
            .unwrap()
            .width = width.clamp(1., 4096.);
    }
    fn repair_active(&mut self) {
        for side in [Side::Left, Side::Right] {
            if !self
                .groups
                .iter()
                .any(|g| g.side == side && Some(g.representative) == self.active[side.index()])
            {
                self.active[side.index()] = self
                    .groups
                    .iter()
                    .find(|g| g.side == side)
                    .map(|g| g.representative);
            }
        }
    }
    fn detach(&mut self, id: PanelId) {
        let index = self
            .groups
            .iter()
            .position(|g| g.views.contains(&id))
            .unwrap();
        let g = &mut self.groups[index];
        g.views.retain(|v| *v != id);
        if g.views.is_empty() {
            self.groups.remove(index);
        } else if g.representative == id {
            let replacement = g.views[0];
            if self.active[g.side.index()] == Some(id) {
                self.active[g.side.index()] = Some(replacement);
            }
            g.representative = replacement;
        }
    }
    /// Move one view into its own group. No unrelated view changes side.
    pub fn split_to(&mut self, id: PanelId, side: Side) {
        let width = self.group(id).width;
        if self.group(id).views.len() == 1 {
            self.groups
                .iter_mut()
                .find(|g| g.views.contains(&id))
                .unwrap()
                .side = side;
        } else {
            self.detach(id);
            self.groups.push(PanelGroup {
                representative: id,
                side,
                views: vec![id],
                width,
            });
        }
        self.repair_active();
        self.select(id);
    }
    pub fn move_group(&mut self, id: PanelId, side: Side) {
        let g = self
            .groups
            .iter_mut()
            .find(|g| g.views.contains(&id))
            .unwrap();
        g.side = side;
        let representative = g.representative;
        self.repair_active();
        self.active[side.index()] = Some(representative);
    }
    pub fn join(&mut self, id: PanelId, target: PanelId) {
        if self.group(id).representative == self.group(target).representative {
            return;
        }
        self.detach(id);
        self.groups
            .iter_mut()
            .find(|g| g.views.contains(&target))
            .unwrap()
            .views
            .push(id);
        self.repair_active();
        self.select(target);
    }
    pub fn move_to(&mut self, id: PanelId, slot: Slot) {
        let side = if matches!(slot, Slot::LeftTop | Slot::LeftBottom) {
            Side::Left
        } else {
            Side::Right
        };
        if matches!(slot, Slot::LeftBottom | Slot::RightBottom)
            && let Some(target) = self.active[side.index()]
        {
            self.join(id, target);
            return;
        }
        self.split_to(id, side);
    }
    pub fn resize(&mut self, id: PanelId, width: f32, height: f32) -> Result<(), &'static str> {
        if !valid_dimension(width) || !valid_dimension(height) {
            return Err("panel dimensions must be 1..4096 DIP");
        }
        self.set_width(id, width);
        self.get_mut(id).height = height;
        Ok(())
    }
    pub fn geometry(&self, width: f32, height: f32, top: f32, bottom: f32) -> Geometry {
        let width = finite_nonnegative(width);
        let height = finite_nonnegative(height);
        let activity = 44.0_f32.min(width);
        let top = finite_nonnegative(top).min(height);
        let body = (height - top - finite_nonnegative(bottom)).max(0.);
        let available = (width - activity).max(0.);
        let sides = (available - 320.).max(0.);
        let side_width = |side| {
            self.groups
                .iter()
                .find(|g| {
                    g.side == side
                        && Some(g.representative) == self.active[side.index()]
                        && g.views.iter().any(|id| self.displayed(*id))
                })
                .map_or(0., |g| g.width)
        };
        let mut widths = [side_width(Side::Left), side_width(Side::Right)];
        let total = widths[0] + widths[1];
        if total > sides {
            for w in &mut widths {
                *w *= sides / total;
            }
        }
        let mut panels = IDS.map(|id| (id, Rect::default()));
        for g in &self.groups {
            if self.active[g.side.index()] != Some(g.representative) {
                continue;
            }
            let views: Vec<_> = g
                .views
                .iter()
                .copied()
                .filter(|id| self.displayed(*id))
                .collect();
            let collapsed = views.iter().filter(|id| self.get(**id).collapsed).count();
            let header = 30.0_f32.min(body / views.len().max(1) as f32);
            let remaining = (body - header * collapsed as f32).max(0.);
            let weights: f32 = views
                .iter()
                .filter(|id| !self.get(**id).collapsed)
                .map(|id| self.get(*id).height)
                .sum();
            let mut y = top;
            for id in views {
                let p = self.get(id);
                let h = if p.collapsed {
                    header
                } else {
                    remaining * p.height / weights.max(1.)
                };
                panels[id as usize].1 = Rect {
                    x: if g.side == Side::Left {
                        activity
                    } else {
                        width - widths[1]
                    },
                    y,
                    width: widths[g.side.index()],
                    height: h,
                };
                y += h;
            }
        }
        Geometry {
            editor: Rect {
                x: activity + widths[0],
                y: top,
                width: (available - widths[0] - widths[1]).max(0.),
                height: body,
            },
            panels,
        }
    }
    pub fn serialize(&self) -> String {
        let active = |side: Side| self.active[side.index()].map_or("", |id| NAMES[id as usize]);
        let mut text = format!(
            "schema_version = 2\nleft_visible = {}\nright_visible = {}\nleft_active = \"{}\"\nright_active = \"{}\"\n",
            self.left_visible,
            self.right_visible,
            active(Side::Left),
            active(Side::Right)
        );
        for g in &self.groups {
            text.push_str(&format!(
                "\n[[group]]\nrepresentative = \"{}\"\nside = \"{}\"\nwidth = {}\nviews = \"{}\"\n",
                NAMES[g.representative as usize],
                if g.side == Side::Left {
                    "left"
                } else {
                    "right"
                },
                g.width,
                g.views
                    .iter()
                    .map(|id| NAMES[*id as usize])
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        for p in &self.panels {
            text.push_str(&format!(
                "\n[[panel]]\nid = \"{}\"\nheight = {}\ncollapsed = {}\nhidden = {}\n",
                NAMES[p.id as usize], p.height, p.collapsed, p.hidden
            ));
        }
        text
    }
    pub fn parse(text: &str) -> Result<Self, &'static str> {
        use std::collections::BTreeMap;
        let mut header = BTreeMap::new();
        let mut records: Vec<(&str, BTreeMap<&str, &str>)> = Vec::new();
        for line in text
            .trim_start_matches('\u{feff}')
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.starts_with('#'))
        {
            if line == "[[panel]]" || line == "[[group]]" {
                records.push((line, BTreeMap::new()));
                continue;
            }
            let (key, value) = line.split_once('=').ok_or("invalid panel line")?;
            let record = if let Some((_, record)) = records.last_mut() {
                record
            } else {
                &mut header
            };
            if record.insert(key.trim(), value.trim()).is_some() {
                return Err("duplicate panel field");
            }
        }
        fn field<'a>(r: &BTreeMap<&str, &'a str>, key: &str) -> Result<&'a str, &'static str> {
            r.get(key).copied().ok_or("missing panel field")
        }
        let boolean = |r: &BTreeMap<&str, &str>, key| {
            field(r, key)?
                .parse::<bool>()
                .map_err(|_| "invalid panel boolean")
        };
        let dimension = |r: &BTreeMap<&str, &str>, key| {
            let n = field(r, key)?
                .parse::<f32>()
                .map_err(|_| "invalid panel dimension")?;
            if valid_dimension(n) {
                Ok(n)
            } else {
                Err("invalid panel dimension")
            }
        };
        let id = |s: &str| {
            NAMES
                .iter()
                .position(|n| *n == s)
                .map(|i| IDS[i])
                .ok_or("unknown panel id")
        };
        let mut result = Self::default();
        for (key, target) in [
            ("left_visible", &mut result.left_visible),
            ("right_visible", &mut result.right_visible),
        ] {
            if header.contains_key(key) {
                *target = boolean(&header, key)?;
            }
        }
        match field(&header, "schema_version")? {
            "1" => {
                if header
                    .keys()
                    .any(|k| !["schema_version", "left_visible", "right_visible"].contains(k))
                    || records.len() != 4
                {
                    return Err("invalid legacy panel schema");
                }
                let mut used = [false; 4];
                let mut slots = [None; 4];
                let mut widths = [0.; 4];
                for (kind, r) in records {
                    if kind != "[[panel]]" || r.len() != 6 {
                        return Err("incomplete legacy panel record");
                    }
                    let panel = id(unquote(field(&r, "id")?).ok_or("invalid id")?)?;
                    if panel == PanelId::Search || used[panel as usize] {
                        return Err("duplicate legacy panel id");
                    }
                    used[panel as usize] = true;
                    let slot = SLOT_NAMES
                        .iter()
                        .position(|n| Some(*n) == unquote(field(&r, "slot").unwrap_or("")))
                        .ok_or("unknown panel slot")?;
                    if slots[slot].replace(panel).is_some() {
                        return Err("duplicate legacy slot");
                    }
                    widths[slot] = dimension(&r, "width")?;
                    let p = result.get_mut(panel);
                    p.height = dimension(&r, "height")?;
                    p.hidden = boolean(&r, "hidden")?;
                    p.collapsed = boolean(&r, "collapsed")?;
                }
                result.groups.clear();
                for (side, offset) in [(Side::Left, 0), (Side::Right, 2)] {
                    let views = vec![slots[offset].unwrap(), slots[offset + 1].unwrap()];
                    let representative = if views.contains(&PanelId::Explorer) {
                        PanelId::Explorer
                    } else {
                        views[0]
                    };
                    result.groups.push(PanelGroup {
                        representative,
                        side,
                        views,
                        width: widths[offset].max(widths[offset + 1]),
                    });
                    result.active[side.index()] = Some(representative);
                }
                result.groups.push(PanelGroup {
                    representative: PanelId::Search,
                    side: Side::Left,
                    views: vec![PanelId::Search],
                    width: widths[0].max(widths[1]),
                });
            }
            "2" => {
                if header.len() != 5 {
                    return Err("invalid group header");
                }
                result.groups.clear();
                let mut seen = [false; 5];
                let mut states = [false; 5];
                for (kind, r) in records {
                    if kind == "[[group]]" {
                        if r.len() != 4 {
                            return Err("invalid group record");
                        }
                        let representative = id(unquote(field(&r, "representative")?)
                            .ok_or("invalid representative")?)?;
                        let side = match unquote(field(&r, "side")?) {
                            Some("left") => Side::Left,
                            Some("right") => Side::Right,
                            _ => return Err("invalid side"),
                        };
                        let mut views = Vec::new();
                        for name in unquote(field(&r, "views")?)
                            .ok_or("invalid views")?
                            .split(',')
                        {
                            let view = id(name)?;
                            if seen[view as usize] {
                                return Err("duplicate view membership");
                            }
                            seen[view as usize] = true;
                            views.push(view);
                        }
                        if !views.contains(&representative) {
                            return Err("representative must belong to group");
                        }
                        result.groups.push(PanelGroup {
                            representative,
                            side,
                            views,
                            width: dimension(&r, "width")?,
                        });
                    } else {
                        if r.len() != 4 {
                            return Err("invalid view record");
                        }
                        let view = id(unquote(field(&r, "id")?).ok_or("invalid id")?)?;
                        if states[view as usize] {
                            return Err("duplicate view state");
                        }
                        states[view as usize] = true;
                        let p = result.get_mut(view);
                        p.height = dimension(&r, "height")?;
                        p.hidden = boolean(&r, "hidden")?;
                        p.collapsed = boolean(&r, "collapsed")?;
                    }
                }
                if seen.contains(&false) || states.contains(&false) {
                    return Err("all views required");
                }
                for (side, key) in [(Side::Left, "left_active"), (Side::Right, "right_active")] {
                    let name = unquote(field(&header, key)?).ok_or("invalid active group")?;
                    result.active[side.index()] = if name.is_empty() {
                        None
                    } else {
                        Some(id(name)?)
                    };
                    if let Some(active) = result.active[side.index()] {
                        if !result
                            .groups
                            .iter()
                            .any(|g| g.representative == active && g.side == side)
                        {
                            return Err("active group on wrong side");
                        }
                    } else if result.groups.iter().any(|g| g.side == side) {
                        return Err("missing active group");
                    }
                }
            }
            _ => return Err("unsupported panel schema"),
        }
        Ok(result)
    }
}
fn unquote(s: &str) -> Option<&str> {
    s.strip_prefix('"')?.strip_suffix('"')
}
fn valid_dimension(n: f32) -> bool {
    n.is_finite() && (1.0..=4096.0).contains(&n)
}
fn finite_nonnegative(n: f32) -> f32 {
    if n.is_finite() { n.max(0.) } else { 0. }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn legacy() -> String {
        let mut s = "schema_version = 1\nleft_visible = false\nright_visible = true\n".to_owned();
        for i in 0..4 {
            s.push_str(&format!("[[panel]]\nid = \"{}\"\nslot = \"{}\"\nwidth = {}\nheight = {}\ncollapsed = {}\nhidden = {}\n", NAMES[i], SLOT_NAMES[i], 200 + i * 10, 100 + i * 50, i == 1, i == 3));
        }
        s
    }
    #[test]
    fn fresh_defaults_are_left_singletons_and_empty_right_releases_width() {
        let l = PanelLayout::default();
        assert_eq!(l.groups.len(), 5);
        assert!(
            l.groups
                .iter()
                .all(|g| g.side == Side::Left && g.views.len() == 1)
        );
        assert_eq!(l.active, [Some(PanelId::Explorer), None]);
        let g = l.geometry(1440., 900., 36., 24.);
        assert_eq!(g.editor.width, 1156.);
        assert_eq!(g.panels.iter().filter(|(_, r)| r.width > 0.).count(), 1);
        assert_eq!(PanelLayout::parse(&l.serialize()).unwrap(), l);
    }
    #[test]
    fn legacy_migration_preserves_groups_flags_widths_weights_and_is_idempotent() {
        let l = PanelLayout::parse(&legacy()).unwrap();
        assert_eq!(
            l.group(PanelId::Explorer).views,
            [PanelId::Explorer, PanelId::Calendar]
        );
        assert_eq!(
            l.group(PanelId::Outline).views,
            [PanelId::Outline, PanelId::Git]
        );
        assert_eq!(l.group(PanelId::Explorer).width, 210.);
        assert_eq!(l.group(PanelId::Outline).width, 230.);
        assert_eq!(l.get(PanelId::Calendar).height, 150.);
        assert!(l.get(PanelId::Calendar).collapsed);
        assert!(l.get(PanelId::Git).hidden);
        assert!(!l.left_visible);
        let text = l.serialize();
        let loaded = PanelLayout::parse(&text).unwrap();
        assert_eq!(loaded, l);
        assert_eq!(loaded.serialize(), text);
    }
    #[test]
    fn search_switch_restores_whole_group_and_right_is_unchanged() {
        let mut l = PanelLayout::parse(&legacy()).unwrap();
        l.left_visible = true;
        let right = l.group(PanelId::Outline).clone();
        let right_active = l.active[1];
        let before = l.geometry(1440., 900., 36., 24.);
        l.select(PanelId::Search);
        let g = l.geometry(1440., 900., 36., 24.);
        assert_eq!(g.panels[0].1, Rect::default());
        assert_eq!(g.panels[1].1, Rect::default());
        assert!(g.panels[4].1.height > 0.);
        assert_eq!(l.group(PanelId::Outline), &right);
        assert_eq!(l.active[1], right_active);
        assert_eq!(before.panels[2], g.panels[2]);
        l.select(PanelId::Explorer);
        assert_eq!(l.geometry(1440., 900., 36., 24.).panels, before.panels);
    }
    #[test]
    fn explicit_join_split_and_move_never_swap_duplicate_or_strand_views() {
        let mut l = PanelLayout::default();
        l.join(PanelId::Calendar, PanelId::Explorer);
        assert_eq!(l.group(PanelId::Calendar).representative, PanelId::Explorer);
        l.split_to(PanelId::Outline, Side::Right);
        l.join(PanelId::Git, PanelId::Outline);
        let left_active = l.active[0];
        l.select(PanelId::Git);
        assert_eq!(l.active[0], left_active);
        l.split_to(PanelId::Calendar, Side::Right);
        assert_eq!(l.side(PanelId::Outline), Side::Right);
        assert_eq!(l.side(PanelId::Explorer), Side::Left);
        l.move_group(PanelId::Outline, Side::Left);
        assert_eq!(l.side(PanelId::Git), Side::Left);
        for view in IDS {
            assert_eq!(
                l.groups
                    .iter()
                    .flat_map(|g| &g.views)
                    .filter(|id| **id == view)
                    .count(),
                1
            );
        }
        l.split_to(PanelId::Calendar, Side::Left);
        assert_eq!(l.active[1], None);
        assert!(l.groups.iter().all(|g| !g.views.is_empty()));
        assert_eq!(PanelLayout::parse(&l.serialize()).unwrap(), l);
    }
    #[test]
    fn rejects_malformed_or_duplicate_membership_without_reset() {
        let text = PanelLayout::default().serialize();
        for bad in [
            text.replacen("views = \"calendar\"", "views = \"explorer\"", 1),
            text.replacen("width = 240", "width = NaN", 1),
            text.replacen("right_active = \"\"", "right_active = \"explorer\"", 1),
            text.replacen("schema_version = 2", "schema_version = 3", 1),
            legacy().replacen("left_bottom", "left_top", 1),
        ] {
            assert!(PanelLayout::parse(&bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn geometry_reserves_editor_and_region_toggle_preserves_groups() {
        let mut l = PanelLayout::parse(&legacy()).unwrap();
        l.left_visible = true;
        let groups = l.groups.clone();
        for width in [0., 20., 250., 600., 1440.] {
            for (left, right) in [(true, true), (false, true), (true, false), (false, false)] {
                l.left_visible = left;
                l.right_visible = right;
                let g = l.geometry(width, 900., 76., 24.);
                assert!(g.editor.width >= (width - 44.).clamp(0., 320.) - 0.01);
                for (_, r) in g.panels {
                    assert!(r.x >= 0. && r.x + r.width <= width + 0.01 && r.height >= 0.);
                }
                assert_eq!(l.groups, groups);
                assert_eq!(PanelLayout::parse(&l.serialize()).unwrap(), l);
            }
        }
    }
}
