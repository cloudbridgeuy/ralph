use super::*;

const ROOM: Room = Room {
    cell_px: (10, 20),
    cols: 80,
    rows: 40,
};

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| name.to_string()).collect()
}

#[test]
fn layout_width_lets_text_match_the_cell_height() {
    assert_eq!(layout_width(&ROOM, 16.0), 640.0);
    let tall_cells = Room {
        cell_px: (8, 16),
        ..ROOM
    };
    assert_eq!(layout_width(&tall_cells, 16.0), 640.0);
}

#[test]
fn a_small_diagram_grows_only_until_text_is_cell_height() {
    let fit = picture_fit((200.0, 100.0), 16.0, &ROOM, 0.6).unwrap();
    assert_eq!(fit.scale, 1.25);
    assert_eq!((fit.width_px, fit.height_px), (250, 125));
}

#[test]
fn a_wide_diagram_shrinks_to_the_columns() {
    let fit = picture_fit((1000.0, 100.0), 16.0, &ROOM, 0.6).unwrap();
    assert_eq!(fit.scale, 0.8);
    assert_eq!((fit.width_px, fit.height_px), (800, 80));
}

#[test]
fn a_tall_diagram_shrinks_to_the_rows() {
    let fit = picture_fit((100.0, 1000.0), 16.0, &ROOM, 0.6).unwrap();
    assert_eq!(fit.scale, 0.8);
    assert_eq!((fit.width_px, fit.height_px), (80, 800));
}

#[test]
fn text_below_the_ratio_of_the_cell_height_falls_back() {
    assert_eq!(picture_fit((4000.0, 300.0), 16.0, &ROOM, 0.6), None);
    assert_eq!(picture_fit((300.0, 4000.0), 16.0, &ROOM, 0.6), None);
    let at_limit = picture_fit((1066.0, 100.0), 16.0, &ROOM, 0.6);
    assert!(at_limit.is_some());
    assert_eq!(picture_fit((1067.0, 100.0), 16.0, &ROOM, 0.6), None);
}

#[test]
fn empty_or_broken_sizes_have_no_picture() {
    for size in [(0.0, 10.0), (10.0, 0.0), (-1.0, 10.0), (f64::NAN, 10.0)] {
        assert_eq!(picture_fit(size, 16.0, &ROOM, 0.6), None, "{size:?}");
    }
    assert_eq!(picture_fit((10.0, 10.0), 0.0, &ROOM, 0.6), None);
    let no_room = Room { cols: 0, ..ROOM };
    assert_eq!(picture_fit((10.0, 10.0), 16.0, &no_room, 0.6), None);
}

#[test]
fn a_tiny_fit_still_has_one_pixel() {
    let fit = picture_fit((0.1, 0.1), 16.0, &ROOM, 0.0).unwrap();
    assert_eq!((fit.width_px, fit.height_px), (1, 1));
}

#[test]
fn the_first_wanted_font_that_is_installed_wins() {
    let wanted = names(&["Helvetica Neue", "Arial"]);
    assert_eq!(
        first_installed(&wanted, &names(&["arial", "Helvetica Neue"])),
        Some("Helvetica Neue")
    );
    assert_eq!(first_installed(&wanted, &names(&["ARIAL"])), Some("Arial"));
    assert_eq!(first_installed(&wanted, &names(&["Times"])), None);
}

#[test]
fn css_families_quote_names_with_spaces() {
    assert_eq!(
        css_families(&names(&["Helvetica Neue", "Arial"])),
        "'Helvetica Neue', Arial"
    );
    assert_eq!(css_families(&[]), "");
}
