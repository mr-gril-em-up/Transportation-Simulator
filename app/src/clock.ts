// One game clock for the whole app. Zoom never touches it.
// At 1x, 1 real second = 1 game minute. Time-lapse multiplies that everywhere.

export const LAPSE_STEPS = [1, 2, 10, 60] as const;

export class GameClock {
  minutes = 6 * 60; // game minutes since Day 1 00:00
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

  /** Advance by real elapsed seconds. */
  tick(realSeconds: number): void {
    if (!this.paused) this.minutes += realSeconds * this.speed;
  }

  label(): string {
    const m = Math.floor(this.minutes);
    const day = Math.floor(m / 1440) + 1;
    const hh = String(Math.floor((m % 1440) / 60)).padStart(2, "0");
    const mm = String(m % 60).padStart(2, "0");
    return `Day ${day} ${hh}:${mm}`;
  }
}
