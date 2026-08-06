use dioxus::prelude::*;
use tw_merge::tw_merge;

#[derive(Default, Clone, PartialEq)]
pub enum ButtonVariant {
    #[default]
    Default,
    Outline,
    Secondary,
    Ghost,
}

#[derive(Default, Clone, PartialEq)]
pub enum ButtonSize {
    #[default]
    Default,
    Sm,
    IconSm,
}

impl ButtonVariant {
    fn class(&self) -> &'static str {
        match self {
            Self::Default => "bg-primary text-primary-foreground shadow-xs hover:bg-primary/90",
            Self::Outline => {
                "border bg-background shadow-xs hover:bg-accent hover:text-accent-foreground"
            }
            Self::Secondary => {
                "bg-secondary text-secondary-foreground shadow-xs hover:bg-secondary/80"
            }
            Self::Ghost => "hover:bg-accent hover:text-accent-foreground",
        }
    }
}

impl ButtonSize {
    fn class(&self) -> &'static str {
        match self {
            Self::Default => "h-9 px-4 py-2",
            Self::Sm => "h-8 rounded-md px-3 text-xs",
            Self::IconSm => "size-8 rounded-md",
        }
    }
}

#[component]
pub fn Button(
    #[props(into, optional)] class: Option<String>,
    #[props(default = ButtonVariant::default())] variant: ButtonVariant,
    #[props(default = ButtonSize::default())] size: ButtonSize,
    #[props(optional)] disabled: bool,
    #[props(into, optional)] button_type: Option<String>,
    #[props(optional)] onclick: Option<EventHandler<MouseEvent>>,
    #[props(into, optional)] aria_label: Option<String>,
    #[props(into, optional)] title: Option<String>,
    children: Element,
) -> Element {
    let class = tw_merge!(
        "inline-flex w-fit shrink-0 items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium outline-none transition-all hover:cursor-pointer disabled:pointer-events-none disabled:opacity-50 focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50",
        variant.class(),
        size.class(),
        class.as_deref().unwrap_or("")
    );

    rsx! {
        button {
            class,
            r#type: button_type.as_deref().unwrap_or("button"),
            disabled,
            aria_label: aria_label.as_deref(),
            title: title.as_deref(),
            onclick: move |event| {
                if let Some(handler) = &onclick {
                    handler.call(event);
                }
            },
            {children}
        }
    }
}

#[derive(Default, Clone, PartialEq)]
pub enum BadgeVariant {
    #[default]
    Default,
    Outline,
}

impl BadgeVariant {
    fn class(&self) -> &'static str {
        match self {
            Self::Default => {
                "border-transparent bg-primary text-primary-foreground shadow hover:bg-primary/80"
            }
            Self::Outline => "text-foreground",
        }
    }
}

#[component]
pub fn Badge(
    #[props(into, optional)] class: Option<String>,
    #[props(default = BadgeVariant::default())] variant: BadgeVariant,
    children: Element,
) -> Element {
    let class = tw_merge!(
        "inline-flex w-fit max-w-full items-center rounded-md border px-2.5 py-0.5 text-xs font-semibold transition-colors",
        variant.class(),
        class.as_deref().unwrap_or("")
    );
    rsx! { span { class, {children} } }
}

#[component]
pub fn Table(#[props(into, optional)] class: Option<String>, children: Element) -> Element {
    let class = tw_merge!(
        "w-full caption-bottom text-sm",
        class.as_deref().unwrap_or("")
    );
    rsx! {
        div { class: "relative w-full overflow-x-auto",
            table { class, {children} }
        }
    }
}

#[component]
pub fn TableHeader(children: Element) -> Element {
    rsx! { thead { class: "[&_tr]:border-b", {children} } }
}

#[component]
pub fn TableBody(children: Element) -> Element {
    rsx! { tbody { class: "[&_tr:last-child]:border-0", {children} } }
}

#[component]
pub fn TableRow(#[props(into, optional)] class: Option<String>, children: Element) -> Element {
    let class = tw_merge!(
        "border-b transition-colors hover:bg-muted/50",
        class.as_deref().unwrap_or("")
    );
    rsx! { tr { class, {children} } }
}

#[component]
pub fn TableHead(#[props(into, optional)] class: Option<String>, children: Element) -> Element {
    let class = tw_merge!(
        "h-10 px-2 text-left align-middle text-xs font-medium text-muted-foreground whitespace-nowrap",
        class.as_deref().unwrap_or("")
    );
    rsx! { th { class, {children} } }
}

#[component]
pub fn TableCell(#[props(into, optional)] class: Option<String>, children: Element) -> Element {
    let class = tw_merge!("p-2 align-middle", class.as_deref().unwrap_or(""));
    rsx! { td { class, {children} } }
}

#[component]
pub fn Card(#[props(into, optional)] class: Option<String>, children: Element) -> Element {
    let class = tw_merge!(
        "flex flex-col gap-4 rounded-md border bg-card py-6 text-card-foreground shadow-sm",
        class.as_deref().unwrap_or("")
    );
    rsx! { div { class, {children} } }
}

#[component]
pub fn CardHeader(#[props(into, optional)] class: Option<String>, children: Element) -> Element {
    let class = tw_merge!(
        "grid auto-rows-min grid-rows-[auto_auto] items-start gap-1.5 px-6",
        class.as_deref().unwrap_or("")
    );
    rsx! { div { class, {children} } }
}

#[component]
pub fn CardTitle(#[props(into, optional)] class: Option<String>, children: Element) -> Element {
    let class = tw_merge!("font-semibold leading-none", class.as_deref().unwrap_or(""));
    rsx! { h2 { class, {children} } }
}

#[component]
pub fn CardContent(#[props(into, optional)] class: Option<String>, children: Element) -> Element {
    let class = tw_merge!("px-6", class.as_deref().unwrap_or(""));
    rsx! { div { class, {children} } }
}
