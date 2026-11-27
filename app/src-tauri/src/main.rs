// Desktop shell. The web client in app/src does all rendering; the simulation
// is sim-core, exposed here as Tauri commands behind the client's SimClient
// interface (app/src/sim/tauri.ts).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod sim;

fn main() {
    tauri::Builder::default()
        .manage(sim::SimState::start().spawn_clock())
        .invoke_handler(tauri::generate_handler![
            sim::sim_status,
            sim::upsert_route,
            sim::remove_route,
            sim::desire_lines,
            sim::preview_route,
            sim::ledger,
            sim::issue_bond,
            sim::clock,
            sim::set_clock,
            sim::skip_to_next_month,
            sim::results,
            sim::pending_routes
        ])
        .run(tauri::generate_context!())
        .expect("error while running Transpo-Sim");
}
