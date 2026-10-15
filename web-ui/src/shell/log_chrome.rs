//! Log filter chips and facet buttons.

use crate::shell::page::{self, Page};
use crate::shell::present;

pub(super) fn meta(
    page: &Page,
    parent: &web_sys::Element,
    label: &str,
    value: &str,
    clear: Option<&str>,
) {
    let chip = present::node(page, "span", None);
    page::attr(&chip, "class", "log-meta");
    let name = present::node(page, "span", None);
    page::attr(&name, "class", "log-meta-label");
    name.set_text_content(Some(&format!("{label}:")));
    let text = present::node(page, "span", None);
    text.set_text_content(Some(value));
    chip.append_child(&name).ok();
    chip.append_child(&text).ok();
    if let Some(clear) = clear {
        let button = present::node(page, "button", None);
        page::attr(&button, "type", "button");
        page::attr(&button, "data-clear", clear);
        page::attr(&button, "aria-label", &format!("Remove {label} {value}"));
        button.set_text_content(Some("×"));
        chip.append_child(&button).ok();
    }
    parent.append_child(&chip).ok();
}

pub(super) fn facet(page: &Page, parent: &web_sys::Element, id: &str, level: &str, label: &str) {
    button(
        page,
        parent,
        id,
        "data-level",
        level,
        label,
        level.is_empty(),
    );
}

pub(super) fn source_facet(
    page: &Page,
    parent: &web_sys::Element,
    id: &str,
    source: &str,
    label: &str,
    pressed: bool,
) {
    button(page, parent, id, "data-source", source, label, pressed);
}

fn button(
    page: &Page,
    parent: &web_sys::Element,
    id: &str,
    attr: &str,
    value: &str,
    label: &str,
    pressed: bool,
) {
    let button = present::node(page, "button", Some(id));
    page::attr(&button, "type", "button");
    page::attr(&button, "class", "log-facet");
    page::attr(&button, attr, value);
    page::attr(
        &button,
        "aria-pressed",
        if pressed { "true" } else { "false" },
    );
    let mark = present::node(page, "span", None);
    page::attr(&mark, "class", "log-facet-mark");
    page::attr(&mark, "aria-hidden", "true");
    let text = present::node(page, "span", None);
    text.set_text_content(Some(label));
    button.append_child(&mark).ok();
    button.append_child(&text).ok();
    parent.append_child(&button).ok();
}
