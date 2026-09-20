//! The overlay's top-bar applet: a StatusNotifierItem with a dbusmenu of
//! three fixed entries — toggle the touch overlay, toggle the key overlay,
//! quit.
//!
//! The menu never changes, so any SNI host shows it as is. A click flips a
//! flag in `Toggles` and wakes the app loop, which applies it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use otto_kit::AppContext;
use zbus::zvariant::{OwnedValue, Structure, StructureBuilder, Value};
use zbus::{interface, Connection};

const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/MenuBar";
const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";

const MENU_ID_TOUCHES: i32 = 1;
const MENU_ID_KEYS: i32 = 2;
const MENU_ID_QUIT: i32 = 4;

/// What the menu asks for, read by the app loop in `on_update`.
pub struct Toggles {
    pub touches: AtomicBool,
    pub keys: AtomicBool,
    pub quit: AtomicBool,
}

struct Item;

#[interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    #[zbus(property)]
    fn category(&self) -> &str {
        "ApplicationStatus"
    }

    #[zbus(property)]
    fn id(&self) -> &str {
        "input-overlay"
    }

    #[zbus(property)]
    fn title(&self) -> &str {
        "Input Overlay"
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        "Active"
    }

    #[zbus(property)]
    fn icon_name(&self) -> &str {
        ""
    }

    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        vec![touchpad_icon(22), touchpad_icon(44)]
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> &str {
        ""
    }

    #[allow(clippy::type_complexity)]
    #[zbus(property, name = "ToolTip")]
    fn tool_tip(&self) -> (String, Vec<(i32, i32, Vec<u8>)>, String, String) {
        (
            String::new(),
            Vec::new(),
            "Input Overlay".to_string(),
            String::new(),
        )
    }

    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        true
    }

    #[zbus(property, name = "Menu")]
    fn menu(&self) -> zbus::zvariant::ObjectPath<'_> {
        zbus::zvariant::ObjectPath::from_static_str_unchecked(MENU_PATH)
    }

    fn activate(&self, _x: i32, _y: i32) {}
    fn secondary_activate(&self, _x: i32, _y: i32) {}
    fn context_menu(&self, _x: i32, _y: i32) {}
}

struct Menu {
    toggles: Arc<Toggles>,
}

#[interface(name = "com.canonical.dbusmenu")]
impl Menu {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }

    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }

    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    #[allow(clippy::type_complexity)]
    fn get_layout(
        &self,
        parent_id: i32,
        _recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> zbus::fdo::Result<(u32, (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>))> {
        if parent_id != 0 {
            return Ok((1, (parent_id, HashMap::new(), Vec::new())));
        }
        let children = vec![
            menu_entry(MENU_ID_TOUCHES, "Touches"),
            menu_entry(MENU_ID_KEYS, "Key Presses"),
            separator_entry(3),
            menu_entry(MENU_ID_QUIT, "Quit"),
        ];
        let mut root_props = HashMap::new();
        root_props.insert("children-display".to_string(), owned_str("submenu"));
        Ok((1, (0, root_props, children)))
    }

    #[allow(clippy::type_complexity)]
    fn get_group_properties(
        &self,
        _ids: Vec<i32>,
        _property_names: Vec<String>,
    ) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        Vec::new()
    }

    fn get_property(&self, _id: i32, _name: &str) -> zbus::fdo::Result<OwnedValue> {
        Err(zbus::fdo::Error::InvalidArgs("no such property".into()))
    }

    fn event(&self, id: i32, event_id: &str, _data: Value<'_>, _timestamp: u32) {
        if event_id != "clicked" {
            return;
        }
        let flag = match id {
            MENU_ID_TOUCHES => &self.toggles.touches,
            MENU_ID_KEYS => &self.toggles.keys,
            MENU_ID_QUIT => {
                self.toggles.quit.store(true, Ordering::SeqCst);
                AppContext::request_wakeup();
                return;
            }
            _ => return,
        };
        flag.fetch_xor(true, Ordering::SeqCst);
        AppContext::request_wakeup();
    }

    fn event_group(&self, _events: Vec<(i32, String, Value<'_>, u32)>) -> Vec<i32> {
        Vec::new()
    }
}

fn menu_entry(id: i32, label: &str) -> OwnedValue {
    let mut props = HashMap::new();
    props.insert("label".to_string(), owned_str(label));
    props.insert("enabled".to_string(), OwnedValue::from(true));
    props.insert("visible".to_string(), OwnedValue::from(true));
    entry(id, props)
}

fn separator_entry(id: i32) -> OwnedValue {
    let mut props = HashMap::new();
    props.insert("type".to_string(), owned_str("separator"));
    props.insert("visible".to_string(), OwnedValue::from(true));
    entry(id, props)
}

fn entry(id: i32, props: HashMap<String, OwnedValue>) -> OwnedValue {
    let structure: Structure<'_> = StructureBuilder::new()
        .add_field(id)
        .add_field(props)
        .add_field(Vec::<OwnedValue>::new())
        .build();
    OwnedValue::try_from(Value::from(structure)).expect("menu entry is a plain struct")
}

fn owned_str(s: &str) -> OwnedValue {
    OwnedValue::from(zbus::zvariant::Str::from(s.to_string()))
}

/// Publish the applet for the life of the process. Must run inside a tokio
/// runtime.
pub fn spawn(toggles: Arc<Toggles>) {
    tokio::spawn(async move {
        if let Err(e) = publish(toggles).await {
            eprintln!("otto-input-overlay: no top-bar applet: {e}");
        }
    });
}

async fn publish(toggles: Arc<Toggles>) -> zbus::Result<()> {
    use zbus::export::futures_util::StreamExt;

    let name = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
    let conn = zbus::ConnectionBuilder::session()?
        .name(name.as_str())?
        .serve_at(ITEM_PATH, Item)?
        .serve_at(MENU_PATH, Menu { toggles })?
        .build()
        .await?;
    register(&conn, &name).await;

    // A bar that starts later, or restarts, has never seen the registration.
    let dbus = zbus::fdo::DBusProxy::new(&conn).await?;
    let mut changes = dbus.receive_name_owner_changed().await?;
    while let Some(signal) = changes.next().await {
        let Ok(args) = signal.args() else { continue };
        if args.name.as_str() == WATCHER_NAME && args.new_owner.is_some() {
            register(&conn, &name).await;
        }
    }
    Ok(())
}

async fn register(conn: &Connection, name: &str) {
    if let Err(e) = conn
        .call_method(
            Some(WATCHER_NAME),
            WATCHER_PATH,
            Some(WATCHER_NAME),
            "RegisterStatusNotifierItem",
            &(name,),
        )
        .await
    {
        eprintln!("otto-input-overlay: no tray host to register with: {e}");
    }
}

/// A touchpad outline with one finger on it, as ARGB32 in network byte order.
fn touchpad_icon(size: i32) -> (i32, i32, Vec<u8>) {
    let n = size as usize;
    let mut data = vec![0u8; n * n * 4];
    let Some(mut surface) = skia_safe::surfaces::raster_n32_premul((size, size)) else {
        return (size, size, data);
    };
    let s = size as f32 / 22.0;
    let canvas = surface.canvas();
    canvas.clear(skia_safe::Color::TRANSPARENT);
    // Mid gray reads on both a light and a dark bar.
    let color = skia_safe::Color4f::new(0.55, 0.55, 0.58, 1.0);

    let mut stroke = skia_safe::Paint::new(color, None);
    stroke.set_anti_alias(true);
    stroke.set_style(skia_safe::paint::Style::Stroke);
    stroke.set_stroke_width(1.6 * s);
    let pad = skia_safe::Rect::from_ltrb(3.0 * s, 5.0 * s, 19.0 * s, 17.0 * s);
    canvas.draw_round_rect(pad, 2.5 * s, 2.5 * s, &stroke);

    let mut fill = skia_safe::Paint::new(color, None);
    fill.set_anti_alias(true);
    canvas.draw_circle((9.0 * s, 10.5 * s), 2.2 * s, &fill);

    let info = skia_safe::ImageInfo::new(
        (size, size),
        skia_safe::ColorType::RGBA8888,
        skia_safe::AlphaType::Unpremul,
        None,
    );
    let mut rgba = vec![0u8; n * n * 4];
    if surface.read_pixels(&info, &mut rgba, n * 4, (0, 0)) {
        for (argb, px) in data.chunks_exact_mut(4).zip(rgba.chunks_exact(4)) {
            argb.copy_from_slice(&[px[3], px[0], px[1], px[2]]);
        }
    }
    (size, size, data)
}
