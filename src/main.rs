use eframe::{
    egui::{self, CentralPanel, Color32, Context, FontId, Label, RichText},
    App, NativeOptions,
};
use serde::{Deserialize, Serialize};
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
    zvariant::{OwnedValue, Value},
};

#[derive(Deserialize, Serialize, Clone)]
struct Config {
    dbus_service: Option<String>,
    fg_color: String,
    bg_color: String,
    window_x: Option<i32>,
    window_y: Option<i32>,
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
                    let title = &current.title;
                    let artist = &current.artist;

                    // --- Dynamic font sizing ---
                    let max_font_size = 15.0;
                    let min_font_size = 10.0;
                    let padding = 10.0;
                    let target_width = ui.available_width() - padding;

                    let mut font_size = max_font_size;
                    loop {
                        let total_width = ctx.fonts(|fonts| {
                            // Measure title and artist parts separately for accuracy
                            let title_width = fonts
                                .layout_no_wrap(
                                    title.to_string(),
                                    FontId::proportional(font_size),
                                    self.fg_color,
                                )
                                .size()
                                .x;
                            let artist_width = fonts
                                .layout_no_wrap(
                                    format!("{}", artist), // Add the separator for measurement
                                    FontId::proportional(font_size),
                                    self.fg_color,
                                )
                                .size()
                                .x;
                            title_width + artist_width
                        });

                        if total_width <= target_width || font_size <= min_font_size {
                            break;
                        }
                        font_size -= 1.0;
                    }

                    // --- Layout with color emphasis and guaranteed baseline alignment ---
                    let title_color = self.fg_color;
                    let artist_color = Color32::from_gray(180);

                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add_space(5.0);
                        ui.label(
                            RichText::new(title.clone())
                                .font(FontId::proportional(font_size))
                                .color(title_color),
                        );
                        ui.label(
                            RichText::new(format!("{}", artist))
                                .font(FontId::proportional(font_size))
                                .color(artist_color),
                        );
                    });
                } else {
                    let label = Label::new(
                        RichText::new("No media playing")
                            .font(FontId::proportional(16.0))
                            .color(self.fg_color),
                    );
                    ui.with_layout(
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.add_space(5.0);  // 5px left padding
                            ui.add(label);
                        },
                    );
                }
            });
        // Request repaint to allow for updates from the D-Bus thread
        ctx.request_repaint_after(Duration::from_millis(500));
    }
}

fn discover_player(connection: &Connection) -> Result<Option<String>, zbus::Error> {
    let proxy = Proxy::new(
        connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )?;

    let all_names: Vec<String> = proxy.call_method("ListNames", &())?.body().deserialize()?;

    let mpris_players: Vec<String> = all_names
        .into_iter()
        .filter(|name| name.starts_with("org.mpris.MediaPlayer2."))
        .collect();

    if mpris_players.is_empty() {
        return Ok(None);
    }

    let mut playing_player = None;
    let mut paused_player = None;

    for player_name in &mpris_players {
        if let Ok(player_proxy) = Proxy::new(
            connection,
            player_name.as_str(),
            "/org/mpris/MediaPlayer2",
            "org.mpris.MediaPlayer2.Player",
        ) {
            if let Ok(status) = player_proxy.get_property::<String>("PlaybackStatus") {
                if status == "Playing" {
                    playing_player = Some(player_name.clone());
                    break;
                } else if status == "Paused" && paused_player.is_none() {
                    paused_player = Some(player_name.clone());
                }
            }
        }
    }

    Ok(playing_player.or(paused_player).or_else(|| mpris_players.first().cloned()))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load();
    let shared = Arc::new(Mutex::new(AppState { current: None }));

    let shared_clone = Arc::clone(&shared);
    let config_clone = config.clone();
    thread::spawn(move || {
        loop {
            let connection = match Connection::session() {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Failed to connect to D-Bus: {}. Retrying in 5s...", e);
                    thread::sleep(Duration::from_secs(5));
                    continue;
                }
            };

            // --- Main Player Discovery Loop ---
            loop {
                let service_name_to_use = if let Some(name) = &config_clone.dbus_service {
                    Some(name.clone())
                } else {
                    match discover_player(&connection) {
                        Ok(Some(name)) => Some(name),
                        _ => None,
                    }
                };

                if service_name_to_use.is_none() {
                    let mut state = shared_clone.lock().unwrap();
                    state.current = None;
                    thread::sleep(Duration::from_secs(2));
                    continue; // No player found, re-run discovery
                }
                
                let service_name = service_name_to_use.unwrap();

                let proxy_result = Proxy::new(
                    &connection,
                    service_name.as_str(),
                    "/org/mpris/MediaPlayer2",
                    "org.mpris.MediaPlayer2.Player",
                );

                let proxy = match proxy_result {
                    Ok(p) => p,
                    Err(_) => {
                        // Can't create proxy, player might have just closed.
                        // Re-run discovery immediately.
                        thread::sleep(Duration::from_millis(500));
                        continue;
                    }
                };

                // --- Track Info Polling Loop ---
                loop {
                    // First, check the playback status. If not "Playing", or if we get an error,
                    // break out and re-run the discovery to find a new active player.
                    match proxy.get_property::<String>("PlaybackStatus") {
                        Ok(status) if status == "Playing" => {
                            // All good, continue to get metadata.
                        }
                        _ => {
                            // Player is paused, stopped, or has disconnected. Time to find a new one.
                            let mut state = shared_clone.lock().unwrap();
                            state.current = None;
                            break;
                        }
                    }
                    
                    match proxy.get_property::<HashMap<String, Value>>("Metadata") {
                        Ok(metadata) => {
                            let mut title = String::new();
                            let mut artist = String::new();

                            if let Some(title_value) = metadata.get("xesam:title") {
                                if let Ok(s_owned_value) = OwnedValue::try_from(title_value) {
                                    if let Ok(string_val) = TryInto::<String>::try_into(s_owned_value) {
                                        title = string_val;
                                    }
                                }
                            }

                            if let Some(artist_value) = metadata.get("xesam:artist") {
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
                            // This error means the player probably closed unexpectedly.
                            // Break out to re-run discovery.
                            let mut state = shared_clone.lock().unwrap();
                            state.current = None;
                            break;
                        }
                    }
                    thread::sleep(Duration::from_secs(1));
                }
            }
        }
    });

    let fg_color_parsed = Config::parse_color(&config.fg_color);
    let bg_color_parsed = Config::parse_color(&config.bg_color);
    let window_width = 400.0;
    let window_height = 25.0;
    let window_x = config.window_x.unwrap_or(0) as f32;
    let window_y = config.window_y.unwrap_or(1000) as f32;
    
    //println!("Attempting to position window at: x={}, y={}", window_x, window_y);
    
    let native_options = NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([window_width, window_height])
            .with_position([window_x, window_y])
            .with_decorations(false)
            .with_always_on_top()
            .with_resizable(false)
            .with_transparent(true)
            .with_taskbar(false)
            .with_visible(true),
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
