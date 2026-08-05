// Mirrors src-tauri/src/model.rs. The backend serializes camelCase, and every
// field here is semantic — no colors cross the bridge, so the three views
// cannot drift apart.

export type CiState = "pass" | "fail" | "pending" | "none";
export type MineBucket = "blocked" | "ready" | "waiting" | "draft";
export type QueueBucket = "requested" | "no_approval" | "partial";
export type CheckState = "pass" | "fail" | "pending" | "skipped";

export type ReviewState =
  | "changes_requested"
  | "approved"
  | "approved_needs_more"
  | "review_required"
  | "none";

export type EventKind =
  | "ci_fail"
  | "ci_pass"
  | "ci_running"
  | "approved"
  | "changes_requested"
  | "commented"
  | "opened"
  | "review_requested";

export interface Ci {
  state: CiState;
  text: string;
  passed: number;
  failed: number;
  pending: number;
  total: number;
}

export interface Check {
  name: string;
  state: CheckState;
  duration: string;
}

export interface TimelineEntry {
  at: string;
  age: string;
  text: string;
}

export interface MinePr {
  id: string;
  slug: string;
  repo: string;
  number: number;
  url: string;
  title: string;
  createdAt: string;
  age: string;
  bucket: MineBucket;
  ci: Ci;
  review: ReviewState;
  reviewText: string;
  approvedBy: string[];
  conflicting: boolean;
  isDraft: boolean;
  labels: string[];
  checks: Check[];
  timeline: TimelineEntry[];
}

export interface QueuePr {
  id: string;
  slug: string;
  repo: string;
  number: number;
  url: string;
  title: string;
  author: string;
  createdAt: string;
  age: string;
  bucket: QueueBucket;
  ticket: string | null;
  ci: Ci;
  approvals: string[];
  requested: string[];
  conflicting: boolean;
  isDraft: boolean;
  checks: Check[];
  timeline: TimelineEntry[];
  siblingNote: string | null;
}

export interface MergedPr {
  slug: string;
  title: string;
  url: string;
  state: string;
  age: string;
}

export interface Hidden {
  total: number;
  alreadyReviewed: number;
  alreadyApproved: number;
  mine: number;
}

export interface RadarEvent {
  id: string;
  at: string;
  kind: EventKind;
  headline: string;
  detail: string;
  slug: string;
  title: string;
  url: string;
  mine: boolean;
}

export interface MineCounts {
  blocked: number;
  ready: number;
  waiting: number;
  draft: number;
}

export interface QueueCounts {
  requested: number;
  noApproval: number;
  partial: number;
}

export interface Snapshot {
  viewer: string;
  avatarUrl: string;
  teams: string[];
  org: string;
  fetchedAt: string;
  mine: MinePr[];
  mineCounts: MineCounts;
  merged: MergedPr[];
  queue: QueuePr[];
  queueCounts: QueueCounts;
  hidden: Hidden;
  events: RadarEvent[];
}

export type Feed =
  | { status: "loading" }
  | { status: "ready"; data: Snapshot }
  | { status: "error"; data: string };

export interface Config {
  org: string;
  label: string;
  pollSeconds: number;
  notify: boolean;
}

/** UI-only preferences. */
export interface Prefs {
  relativeTime: boolean;
  hideApprovedInQueue: boolean;
}

export const DEFAULT_PREFS: Prefs = {
  relativeTime: true,
  hideApprovedInQueue: false,
};
