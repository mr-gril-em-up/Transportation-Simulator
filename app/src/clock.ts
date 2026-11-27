// The one game clock, as the client displays it. The simulation owns time
// (sim-core's Clock, advanced by the shell's clock worker, F21); this class
// interpolates between polls so the display moves every frame, and is
// overwritten by `sync` on each poll. Zoom never touches it.
// At 1x, 1 real second = 1 game minute. Time-lapse multiplies that everywhere.

/** Time-lapse multipliers the button cycles through, in order. */
export const LAPSE_STEPS = [1, 2, 10, 60] as const;

/** The sim's clock as `SimClient.clock()` returns it. */
export interface ClockState {
  gameSeconds: number; // since day 0 00:00
  timelapse: number;
  paused: boolean;
  day: number; // whole game days since the start (day 0)
  /** 0 to 1 while the day's re-run computes (results are still the previous day's); absent otherwise. */
  dayProgress?: number;
}

/** Display copy of the game clock. Advanced only by real time x time-lapse; zoom never touches it. */
export class GameClock {
  minutes = 5 * 60; // game minutes since day 0 00:00; sim-core starts at 05:00
  paused = false;
  private lapseIdx = 0;

  get speed(): number {
    return LAPSE_STEPS[this.lapseIdx];
  }

  /** Cycle 1x -> 2x -> 10x -> 60x -> 1x. Returns the new speed. */
  cycleLapse(): number {
    this.lapseIdx = (this.lapseIdx + 1) % LAPSE_STEPS.length;
    return this.speed;
  }

  /** Advance by real elapsed seconds (interpolation between syncs). */
  tick(realSeconds: number): void {
    if (!this.paused) this.minutes += realSeconds * this.speed;
  }

  /** Adopt the simulation's clock. */
  sync(s: ClockState): void {
    this.minutes = s.gameSeconds / 60;
    this.paused = s.paused;
    const i = LAPSE_STEPS.indexOf(s.timelapse as (typeof LAPSE_STEPS)[number]);
    if (i >= 0) this.lapseIdx = i;
  }

  /** "Day 1 05:00": days count from 1 for players, from 0 in sim-core. */
  label(): string {
    const m = Math.floor(this.minutes);
    const day = Math.floor(m / 1440) + 1;
    const hh = String(Math.floor((m % 1440) / 60)).padStart(2, "0");
    const mm = String(m % 60).padStart(2, "0");
    return `Day ${day} ${hh}:${mm}`;
  }
}
