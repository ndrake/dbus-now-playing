use eframe::{
    egui::{self, CentralPanel, Color32, Context, FontId, Label, RichText},
    App, NativeOptions,
};
use serde::Deserialize;
use std::{
    collections::HashMap,
    convert::TryInto,
    fs,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use zbus::{
    blocking::{Connection, Proxy},
    names::{BusName, InterfaceName},
    zvariant::{ObjectPath, OwnedValue, Value},
};

#[derive(Deserialize, Clone)]
struct Config {
    dbus_service: String,
    dbus_path: String,
    dbus_interface: String,
    fg_color: String,
    bg_color: String,
}

impl Config {
    fn load() -> Self {
        let content = fs::read_to_string("config.toml")
            .expect("Failed to read config.toml. Please create one.");
        toml::from_str(&content).expect("Failed to parse config.toml")
    }

    fn parse_color(s: &str) -> Color32 {
        let s = s.trim_start_matches('#');
        if s.len() != 6 {
            return Color32::WHITE;
        }
        let r = u8::from_str_radix(&s[0..2], 16).unwrap_or(255);
        let g = u8::from_str_radix(&s[2..4], 16).unwrap_or(255);
        let b = u8::from_str_radix(&s[4..6], 16).unwrap_or(255);
        Color32::from_rgb(r, g, b)
    }
}

struct NowPlaying {
    title: String,
    artist: String,
}

struct AppState {
    current: Option<NowPlaying>,
}

struct NowPlayingApp {
    shared: Arc<Mutex<AppState>>,
    fg_color: Color32,
    bg_color: Color32,
}

impl App for NowPlayingApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        CentralPanel::default()
            .frame(egui::Frame::default().fill(self.bg_color))
            .show(ctx, |ui| {
                if let Some(current) = &self.shared.lock().unwrap().current {
                    let text = format!("{} - {}", current.artist, current.title);
                    let label = Label::new(
                        RichText::new(text)
                            .font(FontId::proportional(24.0))
                            .color(self.fg_color),
                    );
                    ui.with_layout(
                        egui::Layout::centered_and_justified(egui::Direction::TopDown),
                        |ui| {
                            ui.add(label);
                        },
                    );
                } else {
                    let label = Label::new(
                        RichText::new("No media playing")
                            .font(FontId::proportional(18.0))
                            .color(self.fg_color),
                    );
                    ui.with_layout(
                        egui::Layout::centered_and_justified(egui::Direction::TopDown),
                        |ui| {
                            ui.add(label);
                        },
                    );
                }
            });
        // Request repaint to allow for updates from the D-Bus thread
        ctx.request_repaint_after(Duration::from_millis(500));
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load();
    let shared = Arc::new(Mutex::new(AppState { current: None }));

    let shared_clone = Arc::clone(&shared);
    let config_clone = config.clone();
    thread::spawn(move || {
        let connection_result = Connection::session();
        let connection = match connection_result {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Failed to connect to D-Bus: {}", e);
                return;
            }
        };

        let proxy_result = Proxy::new(
            &connection,
            BusName::try_from(config_clone.dbus_service.as_str()).unwrap(),
            ObjectPath::try_from(config_clone.dbus_path.as_str()).unwrap(),
            InterfaceName::try_from(config_clone.dbus_interface.as_str()).unwrap(),
        );

        let proxy = match proxy_result {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Failed to create D-Bus proxy: {}", e);
                return;
            }
        };

        loop {
            match proxy.get_property::<HashMap<String, Value>>("Metadata") {
                Ok(metadata) => {
                    let mut title = String::new();
                    let mut artist = String::new();

                    if let Some(title_value) = metadata.get("xesam:title") {
                        // FIX: Removed redundant .clone()
                        if let Ok(s_owned_value) = OwnedValue::try_from(title_value) {
                            if let Ok(string_val) = TryInto::<String>::try_into(s_owned_value) {
                                title = string_val;
                            }
                        }
                    }

                    if let Some(artist_value) = metadata.get("xesam:artist") {
                        // FIX: Removed redundant .clone()
                        if let Ok(artists_vec_owned_value) = OwnedValue::try_from(artist_value) {
                            if let Ok(artists_vec) =
                                TryInto::<Vec<String>>::try_into(artists_vec_owned_value)
                            {
                                if let Some(first_artist) = artists_vec.first() {
                                    artist = first_artist.clone();
                                }
                            }
                        }
                    }

                    let mut state = shared_clone.lock().unwrap();
                    if !title.is_empty() && !artist.is_empty() {
                        state.current = Some(NowPlaying { title, artist });
                    } else {
                        state.current = None;
                    }
                }
                Err(_) => {
                    // This error can happen if the media player is closed.
                    // We'll just clear the current state.
                    let mut state = shared_clone.lock().unwrap();
                    state.current = None;
                }
            }
            thread::sleep(Duration::from_secs(1));
        }
    });

    let fg_color_parsed = Config::parse_color(&config.fg_color);
    let bg_color_parsed = Config::parse_color(&config.bg_color);
    let native_options = NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([600.0, 80.0])
            .with_decorations(false)
            .with_always_on_top()
            .with_resizable(false)
            .with_transparent(true), // Makes the background transparent
        ..Default::default()
    };

    eframe::run_native(
        "Now Playing",
        native_options,
        Box::new(move |_cc| {
            Box::new(NowPlayingApp {
                shared,
                fg_color: fg_color_parsed,
                bg_color: bg_color_parsed,
            })
        }),
    )?;

    Ok(())
}
