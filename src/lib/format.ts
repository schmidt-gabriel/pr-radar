import type { RadarEvent } from "./types";

const MONTHS = [
  "Jan", "Feb", "Mar", "Apr", "May", "Jun",
  "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/** "Aug 3" — the absolute form behind the relativeTime toggle. */
export function shortDate(iso: string): string {
  const d = new Date(iso);
  return `${MONTHS[d.getMonth()]} ${d.getDate()}`;
}

export function clockTime(iso: string): string {
  const d = new Date(iso);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

/**
 * Ages are computed once in Rust at fetch time. When the user prefers absolute
 * dates we swap in the calendar date instead of recomputing anything.
 */
export function displayAge(age: string, iso: string, relative: boolean): string {
  return relative ? age : shortDate(iso);
}

export function syncedAgo(fetchedAt: string, now: number): string {
  const secs = Math.max(0, Math.round((now - new Date(fetchedAt).getTime()) / 1000));
  if (secs < 60) return `synced ${secs}s ago`;
  const mins = Math.round(secs / 60);
  if (mins < 60) return `synced ${mins}m ago`;
  return `synced ${Math.round(mins / 60)}h ago`;
}

export function dayLabel(iso: string, now: Date): string {
  const d = new Date(iso);
  const startOf = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const days = Math.round((startOf(now) - startOf(d)) / 86_400_000);
  if (days <= 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days < 7) return d.toLocaleDateString(undefined, { weekday: "long" });
  return shortDate(iso);
}

export interface EventDay {
  label: string;
  date: string;
  events: RadarEvent[];
}

/** Group the feed into the day sections the timeline view renders. */
export function groupByDay(events: RadarEvent[], now = new Date()): EventDay[] {
  const days: EventDay[] = [];
  for (const e of events) {
    const label = dayLabel(e.at, now);
    let day = days.find((d) => d.label === label);
    if (!day) {
      day = { label, date: shortDate(e.at), events: [] };
      days.push(day);
    }
    day.events.push(e);
  }
  return days;
}

export function pluralize(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}

export function approvedByLabel(who: string[]): string {
  if (who.length === 0) return "";
  if (who.length === 1) return `Approved by ${who[0]}`;
  return `Approved by ${who[0]} +${who.length - 1}`;
}
