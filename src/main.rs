use eframe::egui;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedValue, Value};

#[derive(Default, Clone)]
struct TrackInfo {
    title: String,
    artist: String,
}

struct App {
    info: Arc<Mutex<TrackInfo>>,
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        let TrackInfo { title, artist } = (*self.info.lock().unwrap()).clone();
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Now Playing:");
            ui.label(format!("Title : {}", title));
            ui.label(format!("Artist: {}", artist));
        });
        ctx.request_repaint_after(Duration::from_millis(500));
    }
}

fn poll(info: Arc<Mutex<TrackInfo>>) {
    let conn = Connection::session().unwrap();
    let proxy = Proxy::new(
        &conn,
        "org.mpris.MediaPlayer2.Supersonic",
        "/org/mpris/MediaPlayer2",
        "org.mpris.MediaPlayer2.Player",
    )
    .unwrap();

    loop {
        let v: OwnedValue = proxy.get_property("Metadata").unwrap();
        let v: Value = v.into();
        let meta: HashMap<String, Value> = v.downcast().unwrap();

        let title = meta
            .get("xesam:title")
            .and_then(|v| v.clone().downcast::<String>())
            .unwrap_or_else(|| "Unknown Title".into());
        let artist = meta
            .get("xesam:artist")
            .and_then(|v| v.clone().downcast::<Vec<String>>())
            .and_then(|v| v.into_iter().next())
            .unwrap_or_else(|| "Unknown Artist".into());

        *info.lock().unwrap() = TrackInfo { title, artist };
        thread::sleep(Duration::from_secs(1));
    }
}

fn main() -> Result<(), eframe::Error> {
    let info = Arc::new(Mutex::new(TrackInfo::default()));
    let info2 = info.clone();

    thread::spawn(move || poll(info2));

    eframe::run_native(
        "Now Playing",
        Default::default(),
        Box::new(|_| Box::new(App { info })),
    )
}
