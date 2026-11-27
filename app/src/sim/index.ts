import type { SimClient } from "./api";
import { MockSim } from "./mock";
import { TauriSim } from "./tauri";

// Inside the desktop app: real sim-core. In a plain browser: the mock, so
// `npm run dev` works without Rust.
export function createSim(): SimClient {
  return "__TAURI_INTERNALS__" in window ? new TauriSim() : new MockSim();
}
