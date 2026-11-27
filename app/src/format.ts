/** Dollars for the HUD: $820k, $34.5M, $1.20B. */
export function money(usd: number): string {
  const a = Math.abs(usd);
  const s = a >= 1e9 ? `${(a / 1e9).toFixed(2)}B` : a >= 1e6 ? `${(a / 1e6).toFixed(1)}M` : `${Math.round(a / 1e3)}k`;
  return `${usd < 0 ? "-" : ""}$${s}`;
}

/** Escape text for innerHTML (names are player-typed). */
export const escapeHtml = (s: string) => s.replace(/[&<>"]/g, (c) => `&#${c.charCodeAt(0)};`);
