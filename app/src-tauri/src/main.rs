// Desktop shell. The web client in app/src does all rendering; the simulation
// runs in the client-side mock until sim-core lands, at which point it is
// exposed here as Tauri commands behind the SimClient interface.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running Transpo-Sim");
}
