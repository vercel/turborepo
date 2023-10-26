use turborepo_ui::{cprintln, GREY, UI};

pub fn logout(ui: &UI) {
    cprintln!(ui, GREY, ">>> Logged out");
}
