use crate::theme;
use eframe::egui::{Align2, Color32, CursorIcon, RichText, Sense, TextStyle, Ui, vec2};
use fx_app::{Explorer, Place, PlaceGroup};

const GROUPS: [PlaceGroup; 2] = [PlaceGroup::Places, PlaceGroup::Drives];

pub fn show(ui: &mut Ui, explorer: &mut Explorer, places: &[Place]) {
    let mut goto = None;
    // The deepest place containing the current path wins, so /mnt/c/x highlights /mnt/c, not /.
    let current = places
        .iter()
        .filter(|p| explorer.active().path().starts_with(&p.path))
        .max_by_key(|p| p.path.components().count())
        .map(|p| p.path.clone());
    for group in GROUPS {
        ui.add_space(8.0);
        ui.label(
            RichText::new(group.title().to_uppercase())
                .small()
                .strong()
                .color(theme::dim()),
        );
        ui.add_space(2.0);
        for place in places.iter().filter(|p| p.group == group) {
            let selected = current.as_ref() == Some(&place.path);
            if row(ui, &place.label, selected) {
                goto = Some(place.path.clone());
            }
        }
    }
    if let Some(path) = goto {
        explorer.navigate(path);
    }
}

fn row(ui: &mut Ui, label: &str, selected: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    let fill = if selected {
        theme::selected()
    } else if resp.hovered() {
        theme::panel_hi()
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 3.0, fill);
    let color = if selected {
        theme::accent()
    } else {
        theme::text()
    };
    painter.text(
        rect.left_center() + vec2(8.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        TextStyle::Body.resolve(ui.style()),
        color,
    );
    resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
}
