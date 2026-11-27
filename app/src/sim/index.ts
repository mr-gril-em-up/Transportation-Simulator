import type { SimClient } from "./api";
import { MockSim } from "./mock";

// Single swap point for the real sim-core backend.
export function createSim(): SimClient {
  return new MockSim();
}
