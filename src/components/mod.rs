pub mod badge;
pub mod button;
pub mod checkbox;
pub mod data_table;
pub mod dialog;
pub mod input;
pub mod textarea;

pub(crate) fn load_deferred_stylesheets() {
    checkbox::load_stylesheet();
    data_table::load_stylesheet();
    dialog::load_stylesheet();
    textarea::load_stylesheet();
}
