import { listen } from "@tauri-apps/api/event";
import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import { AgentOfflineCard } from "../components/AgentOfflineCard";
import { Button } from "../components/Button";
import { Card } from "../components/Card";
import { ErrorMessage, SuccessMessage } from "../components/Feedback";
import { Icon } from "../components/Icon";
import { PageHeader } from "../components/PageHeader";
import { fmt, useI18n } from "../i18n";
import { agentCall, Method, useAction, useAgent } from "../lib/agent";
import { attach, deviceBox, frameFor, percent, sideOf, type Box, type Point } from "../lib/layout";
import type { LayoutDevice, LayoutPosition, LayoutView } from "../lib/types";

export function Layout() {
  const { t } = useI18n();
  const snapshot = useAgent();
  const status = snapshot.connected ? snapshot.status : null;
  const kvm = status?.kvm;

  return (
    <>
      <PageHeader title={t.nav.layout} />
      {!status ? (
        <AgentOfflineCard error={snapshot.error} />
      ) : kvm?.state === "ready" && kvm.layout ? (
        <LayoutEditor view={kvm.layout} />
      ) : (
        <Card>
          <p className="flex items-center gap-3 text-sm text-neutral-600 dark:text-neutral-400">
            <Icon name="monitor" className="size-5" />
            {kvm?.state === "waiting" ? t.layout.waiting : t.layout.notConnected}
          </p>
        </Card>
      )}
    </>
  );
}

type Positions = Record<string, Point>;

function positionsOf(view: LayoutView): Positions {
  return Object.fromEntries(view.devices.map((d) => [d.device_id, { x: d.x, y: d.y }]));
}

function signature(view: LayoutView): string {
  return JSON.stringify([view.revision, view.input_host, view.devices.map((d) => [d.device_id, d.x, d.y, d.monitors])]);
}

interface Drag {
  id: string;
  pointer: number;
  startX: number;
  startY: number;
  origin: Point;
  frame: Box;
  canvasWidth: number;
}

const HIDDEN_EVENT = "window://hidden";
const SHOWN_EVENT = "window://shown";
const NUDGE = 2;
const NUDGE_LARGE = 20;
const GUIDE_THROTTLE_MS = 60;
const GUIDE_REFRESH_MS = 2000;

function useLayoutGuide(positions: LayoutPosition[]) {
  const key = JSON.stringify(positions);
  const latest = useRef(positions);
  const sentAt = useRef(0);
  const timer = useRef<number | undefined>(undefined);
  const hidden = useRef(false);
  latest.current = positions;

  const send = (off = false) => {
    sentAt.current = Date.now();
    const value = off || hidden.current || document.hidden ? null : latest.current;
    void agentCall(Method.LayoutGuide, { positions: value }).catch(() => undefined);
  };

  useEffect(() => {
    window.clearTimeout(timer.current);
    const wait = GUIDE_THROTTLE_MS - (Date.now() - sentAt.current);
    if (wait <= 0) send();
    else timer.current = window.setTimeout(() => send(), wait);
  }, [key]);

  useEffect(() => {
    const interval = window.setInterval(() => send(), GUIDE_REFRESH_MS);
    const onVisibility = () => send();
    document.addEventListener("visibilitychange", onVisibility);
    const unlisten = [
      listen(HIDDEN_EVENT, () => {
        hidden.current = true;
        send();
      }),
      listen(SHOWN_EVENT, () => {
        hidden.current = false;
        send();
      }),
    ];
    return () => {
      window.clearInterval(interval);
      window.clearTimeout(timer.current);
      document.removeEventListener("visibilitychange", onVisibility);
      for (const p of unlisten) void p.then((fn) => fn()).catch(() => undefined);
      send(true);
    };
  }, []);
}

function LayoutEditor({ view }: { view: LayoutView }) {
  const { t } = useI18n();
  const action = useAction();
  const canvasRef = useRef<HTMLDivElement>(null);
  const [positions, setPositions] = useState<Positions>(() => positionsOf(view));
  const [host, setHost] = useState(view.input_host);
  const [drag, setDrag] = useState<Drag | null>(null);
  const [saved, setSaved] = useState(false);
  const base = signature(view);

  useEffect(() => {
    setPositions(positionsOf(view));
    setHost(view.input_host);
    setDrag(null);
  }, [base]);

  const local = view.devices.find((d) => d.is_local) ?? view.devices[0];
  const peer = view.devices.find((d) => d.device_id !== local.device_id) ?? view.devices[1];
  const pos = (d: LayoutDevice) => positions[d.device_id] ?? { x: d.x, y: d.y };

  const liveFrame = useMemo(
    () => frameFor(view.devices.map((d) => deviceBox(d, positions[d.device_id] ?? { x: d.x, y: d.y }))),
    [view.devices, positions],
  );
  const frame = drag?.frame ?? liveFrame;
  const frameW = frame.right - frame.left;
  const frameH = frame.bottom - frame.top;

  const original = positionsOf(view);
  const dirty =
    host !== view.input_host ||
    view.devices.some((d) => {
      const a = positions[d.device_id];
      const b = original[d.device_id];
      return !a || !b || a.x !== b.x || a.y !== b.y;
    });

  const other = (d: LayoutDevice) => (d.device_id === local.device_id ? peer : local);

  const snap = (d: LayoutDevice, p: Point, fine = false) => {
    const anchor = other(d);
    if (!anchor) return p;
    return attach(d, p, anchor, pos(anchor), fine);
  };

  const nudge = (d: LayoutDevice, move: Point) => {
    const p = pos(d);
    update(d, snap(d, { x: p.x + move.x, y: p.y + move.y }, true));
  };

  useLayoutGuide(
    view.devices.map((d) => ({
      device_id: d.device_id,
      ...(drag?.id === d.device_id ? snap(d, pos(d)) : pos(d)),
    })),
  );

  const update = (d: LayoutDevice, p: Point) => {
    setSaved(false);
    setPositions((prev) => ({ ...prev, [d.device_id]: p }));
  };

  const onPointerDown = (d: LayoutDevice) => (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || !canvasRef.current) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    e.currentTarget.focus();
    setDrag({
      id: d.device_id,
      pointer: e.pointerId,
      startX: e.clientX,
      startY: e.clientY,
      origin: pos(d),
      frame: liveFrame,
      canvasWidth: canvasRef.current.getBoundingClientRect().width,
    });
  };

  const onPointerMove = (d: LayoutDevice) => (e: PointerEvent<HTMLDivElement>) => {
    if (!drag || drag.id !== d.device_id || drag.pointer !== e.pointerId) return;
    const unit = (drag.frame.right - drag.frame.left) / Math.max(drag.canvasWidth, 1);
    update(d, {
      x: Math.round(drag.origin.x + (e.clientX - drag.startX) * unit),
      y: Math.round(drag.origin.y + (e.clientY - drag.startY) * unit),
    });
  };

  const onPointerUp = (d: LayoutDevice) => (e: PointerEvent<HTMLDivElement>) => {
    if (!drag || drag.id !== d.device_id || drag.pointer !== e.pointerId) return;
    e.currentTarget.releasePointerCapture(e.pointerId);
    setDrag(null);
    update(d, snap(d, pos(d)));
  };

  const onKeyDown = (d: LayoutDevice) => (e: KeyboardEvent<HTMLDivElement>) => {
    const step = e.shiftKey ? NUDGE_LARGE : NUDGE;
    const delta: Record<string, Point> = {
      ArrowLeft: { x: -step, y: 0 },
      ArrowRight: { x: step, y: 0 },
      ArrowUp: { x: 0, y: -step },
      ArrowDown: { x: 0, y: step },
    };
    const move = delta[e.key];
    if (!move) return;
    e.preventDefault();
    nudge(d, move);
  };

  const peerSide = peer ? sideOf(deviceBox(peer, pos(peer)), deviceBox(local, pos(local))) : null;
  const vertical = peerSide === "left" || peerSide === "right";
  const steps = [
    { amount: -NUDGE_LARGE, label: vertical ? "↑↑" : "←←" },
    { amount: -NUDGE, label: vertical ? "↑" : "←" },
    { amount: NUDGE, label: vertical ? "↓" : "→" },
    { amount: NUDGE_LARGE, label: vertical ? "↓↓" : "→→" },
  ];

  const save = async () => {
    const payload: LayoutPosition[] = view.devices.map((d) => ({ device_id: d.device_id, ...pos(d) }));
    const ok = await action.run(() =>
      agentCall(Method.LayoutSet, { input_host: host, positions: payload }),
    );
    setSaved(ok);
  };

  const reset = () => {
    setPositions(positionsOf(view));
    setHost(view.input_host);
    setSaved(false);
    action.clear();
  };

  return (
    <div className="grid gap-4">
      <Card title={t.layout.title} description={t.layout.intro}>
        <div
          ref={canvasRef}
          dir="ltr"
          role="group"
          aria-label={t.layout.canvasLabel}
          className="relative w-full touch-none select-none overflow-hidden rounded-lg bg-neutral-100 ring-1 ring-black/5 dark:bg-black/25 dark:ring-white/10"
          style={{ aspectRatio: `${frameW} / ${frameH}` }}
        >
          {view.devices.map((d) => (
            <DeviceBlock
              key={d.device_id}
              device={d}
              box={deviceBox(d, pos(d))}
              frame={frame}
              dragging={drag?.id === d.device_id}
              isHost={host === d.device_id}
              onPointerDown={onPointerDown(d)}
              onPointerMove={onPointerMove(d)}
              onPointerUp={onPointerUp(d)}
              onKeyDown={onKeyDown(d)}
            />
          ))}
        </div>

        <div className="mt-5 grid gap-3 rounded-lg bg-white/60 px-4 py-3 ring-1 ring-black/5 dark:bg-white/5 dark:ring-white/10">
          <p className="flex items-center gap-2 text-sm font-medium text-neutral-900 dark:text-neutral-100">
            <Icon name="monitor" className="size-4" />
            {t.layout.guideTitle}
          </p>
          <p className="text-xs text-neutral-600 dark:text-neutral-400">{t.layout.guideHint}</p>
          {peer && (
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-sm text-neutral-700 dark:text-neutral-300">
                {fmt(t.layout.nudgeTitle, { name: peer.device_name })}
              </span>
              <span dir="ltr" className="flex flex-wrap gap-2">
                {steps.map((step) => (
                  <button
                    key={step.amount}
                    type="button"
                    aria-label={fmt(vertical ? t.layout.nudgeVertical : t.layout.nudgeHorizontal, {
                      name: peer.device_name,
                      amount: step.amount,
                    })}
                    onClick={() => nudge(peer, vertical ? { x: 0, y: step.amount } : { x: step.amount, y: 0 })}
                    className="min-w-10 rounded-md bg-white/80 px-2.5 py-1.5 text-sm font-medium text-neutral-800 ring-1 ring-black/10 transition-colors hover:bg-white focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-500 dark:bg-white/10 dark:text-neutral-100 dark:ring-white/10 dark:hover:bg-white/15"
                  >
                    {step.label}
                  </button>
                ))}
              </span>
            </div>
          )}
        </div>
      </Card>

      <Card title={t.layout.hostTitle} description={t.layout.hostHint}>
        <div className="grid gap-3 sm:grid-cols-2" role="radiogroup" aria-label={t.layout.hostTitle}>
          {view.devices.map((d) => {
            const checked = host === d.device_id;
            return (
              <button
                key={d.device_id}
                type="button"
                role="radio"
                aria-checked={checked}
                onClick={() => {
                  setSaved(false);
                  setHost(d.device_id);
                }}
                className={`flex items-center gap-3 rounded-lg px-4 py-3 text-start ring-1 transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-500 ${
                  checked
                    ? "bg-indigo-50 ring-2 ring-indigo-500 dark:bg-indigo-500/15"
                    : "bg-white/60 ring-black/10 hover:bg-white dark:bg-white/5 dark:ring-white/10 dark:hover:bg-white/10"
                }`}
              >
                <Icon name="keyboard" className="size-5 text-neutral-600 dark:text-neutral-300" />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-sm font-medium text-neutral-900 dark:text-neutral-50">
                    {d.device_name}
                  </span>
                  <span className="block text-xs text-neutral-500 dark:text-neutral-400">
                    {d.portable ? t.layout.portable : t.layout.desktop}
                    {d.is_local ? ` · ${t.layout.thisDevice}` : ""}
                  </span>
                </span>
                <span
                  className={`flex size-4 shrink-0 items-center justify-center rounded-full ring-2 ${
                    checked ? "ring-indigo-600 dark:ring-indigo-400" : "ring-neutral-300 dark:ring-neutral-600"
                  }`}
                >
                  {checked && <span className="size-2 rounded-full bg-indigo-600 dark:bg-indigo-400" />}
                </span>
              </button>
            );
          })}
        </div>
      </Card>

      {(dirty || saved || action.error) && (
        <div className="grid gap-3">
          {action.error && <ErrorMessage error={action.error} onDismiss={action.clear} />}
          {saved && !dirty && <SuccessMessage text={t.layout.saved} />}
          {dirty && (
            <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg bg-amber-50 px-4 py-3 ring-1 ring-amber-200 dark:bg-amber-500/10 dark:ring-amber-500/30">
              <p className="text-sm text-amber-900 dark:text-amber-200">{t.layout.unsaved}</p>
              <div className="flex gap-2">
                <Button variant="ghost" disabled={action.busy} onClick={reset}>
                  {t.common.discard}
                </Button>
                <Button variant="primary" icon="check" busy={action.busy} onClick={() => void save()}>
                  {t.common.save}
                </Button>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

interface DeviceBlockProps {
  device: LayoutDevice;
  box: Box;
  frame: Box;
  dragging: boolean;
  isHost: boolean;
  onPointerDown: (e: PointerEvent<HTMLDivElement>) => void;
  onPointerMove: (e: PointerEvent<HTMLDivElement>) => void;
  onPointerUp: (e: PointerEvent<HTMLDivElement>) => void;
  onKeyDown: (e: KeyboardEvent<HTMLDivElement>) => void;
}

function DeviceBlock({ device, box, frame, dragging, isHost, ...handlers }: DeviceBlockProps) {
  const { t } = useI18n();
  const frameW = frame.right - frame.left;
  const frameH = frame.bottom - frame.top;
  const boxW = Math.max(box.right - box.left, 1);
  const boxH = Math.max(box.bottom - box.top, 1);
  const originX = box.left - Math.min(...device.monitors.map((m) => m.x));
  const originY = box.top - Math.min(...device.monitors.map((m) => m.y));
  const tone = device.is_local
    ? "bg-indigo-100/90 ring-indigo-400 text-indigo-950 dark:bg-indigo-500/25 dark:ring-indigo-400/70 dark:text-indigo-50"
    : "bg-emerald-100/90 ring-emerald-400 text-emerald-950 dark:bg-emerald-500/25 dark:ring-emerald-400/70 dark:text-emerald-50";

  return (
    <div
      tabIndex={0}
      role="button"
      aria-roledescription="draggable"
      aria-label={device.device_name}
      onPointerDown={handlers.onPointerDown}
      onPointerMove={handlers.onPointerMove}
      onPointerUp={handlers.onPointerUp}
      onPointerCancel={handlers.onPointerUp}
      onKeyDown={handlers.onKeyDown}
      className={`absolute rounded-md focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-500 ${
        dragging ? "z-10 cursor-grabbing opacity-90" : "cursor-grab"
      }`}
      style={{
        left: percent(box.left, frame.left, frameW),
        top: percent(box.top, frame.top, frameH),
        width: `${(boxW / frameW) * 100}%`,
        height: `${(boxH / frameH) * 100}%`,
      }}
    >
      {device.monitors.map((m, i) => (
        <div
          key={`${m.name}-${i}`}
          className={`absolute flex flex-col items-center justify-center overflow-hidden rounded-sm p-1 text-center ring-2 ${tone}`}
          style={{
            left: percent(originX + m.x, box.left, boxW),
            top: percent(originY + m.y, box.top, boxH),
            width: `${(m.width / boxW) * 100}%`,
            height: `${(m.height / boxH) * 100}%`,
          }}
        >
          {m.primary && (
            <span className="flex max-w-full items-center gap-1 truncate text-xs font-semibold">
              {isHost && <Icon name="keyboard" className="size-3.5" />}
              <span className="truncate">{device.device_name}</span>
            </span>
          )}
          <span className="max-w-full truncate text-xs opacity-75">
            {m.primary ? t.layout.primary : m.name} · {fmt(t.layout.scale, { scale: m.scale_percent })}
            {m.diagonal_inch != null && ` · ${m.diagonal_inch}"`}
          </span>
        </div>
      ))}
    </div>
  );
}
