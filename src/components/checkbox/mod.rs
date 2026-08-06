mod component;
pub use component::*;
pub use dioxus_primitives::checkbox::CheckboxState;

pub fn checkbox_state(checked: bool) -> CheckboxState {
    if checked {
        CheckboxState::Checked
    } else {
        CheckboxState::Unchecked
    }
}

pub fn checkbox_is_checked(state: CheckboxState) -> bool {
    bool::from(state)
}
