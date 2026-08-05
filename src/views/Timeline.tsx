// 1c — Timeline-first. Built around "I never know when each thing happened":
// every row carries a real GitHub timestamp, grouped by day.

import { useMemo, useState } from "react";
import { open } from "../lib/feed";
import { clockTime, groupByDay } from "../lib/format";
import type { Prefs, Snapshot } from "../lib/types";
import { Empty, EVENT_GLYPH, eventState } from "../ui/atoms";

type Scope = "all" | "mine";

export default function Timeline({ snap }: { snap: Snapshot; prefs: Prefs }) {
  const [scope, setScope] = useState<Scope>("all");

  const days = useMemo(() => {
    const events = scope === "mine" ? snap.events.filter((e) => e.mine) : snap.events;
    return groupByDay(events);
  }, [snap.events, scope]);

  return (
    <div className="timeline">
      <div className="timeline-head">
        <div className="big-stat">
          <span className="n s-blocked">{snap.mineCounts.blocked}</span>
          <span className="k">blocked</span>
        </div>
        <div className="big-stat">
          <span className="n s-ready">{snap.mineCounts.ready}</span>
          <span className="k">ready to merge</span>
        </div>
        <div className="big-stat">
          <span className="n" style={{ color: "var(--text-body)" }}>
            {snap.queue.length}
          </span>
          <span className="k">to review</span>
        </div>

        <div className="segmented" style={{ marginLeft: "auto" }}>
          <button aria-selected={scope === "all"} onClick={() => setScope("all")}>
            Everything
          </button>
          <button aria-selected={scope === "mine"} onClick={() => setScope("mine")}>
            Only mine
          </button>
        </div>
      </div>

      <div className="scroll" style={{ flex: 1, minHeight: 0 }}>
        {days.length === 0 && (
          <Empty glyph="◌">
            Nothing has happened in the last two weeks.
            <br />
            That is either peace or a very quiet repo.
          </Empty>
        )}
        {days.map((day) => (
          <div key={day.label}>
            <div className="day-head">
              <span className="label">{day.label}</span>
              <span className="rule" />
              <span className="date">{day.date}</span>
            </div>
            {day.events.map((e) => (
              <button key={e.id} className="ev-row" onClick={() => open(e.url)}>
                <span className="at">{clockTime(e.at)}</span>
                <span className={`icon icon-${e.kind}`}>{EVENT_GLYPH[e.kind]}</span>
                <div className="body">
                  <div className="line">
                    <b>{e.headline}</b>{" "}
                    {e.detail && <span className="ev-detail">{e.detail}</span>}
                  </div>
                  <div className="facts">
                    <span className="slug">{e.slug}</span>
                    <span className="title">{e.title}</span>
                  </div>
                </div>
                <span className={`state s-${e.kind}`}>{eventState(e.kind, e.mine)}</span>
              </button>
            ))}
          </div>
        ))}
      </div>

      <div className="foot-bar">
        <span>Polls every 60s · notifies on CI failure, approval and review requests</span>
        <span>
          {snap.events.length} events · last 14 days
        </span>
      </div>
    </div>
  );
}
