import type { LayoutDevice } from "./types";

export interface Point {
  x: number;
  y: number;
}

export interface Box {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

export type Side = "left" | "right" | "above" | "below";

const MIN_OVERLAP_RATIO = 0.15;
const SNAP_RATIO = 0.06;
const MIN_ASPECT = 0.3;
const MAX_ASPECT = 0.7;
const FRAME_PADDING = 0.08;

export function deviceBox(device: LayoutDevice, pos: Point): Box {
  if (device.monitors.length === 0) {
    return { left: pos.x, top: pos.y, right: pos.x + 1, bottom: pos.y + 1 };
  }
  return device.monitors.reduce<Box>(
    (box, m) => ({
      left: Math.min(box.left, pos.x + m.x),
      top: Math.min(box.top, pos.y + m.y),
      right: Math.max(box.right, pos.x + m.x + m.width),
      bottom: Math.max(box.bottom, pos.y + m.y + m.height),
    }),
    { left: Infinity, top: Infinity, right: -Infinity, bottom: -Infinity },
  );
}

const width = (b: Box) => b.right - b.left;
const height = (b: Box) => b.bottom - b.top;

export function frameFor(boxes: Box[]): Box {
  const union = boxes.reduce<Box>(
    (u, b) => ({
      left: Math.min(u.left, b.left),
      top: Math.min(u.top, b.top),
      right: Math.max(u.right, b.right),
      bottom: Math.max(u.bottom, b.bottom),
    }),
    { left: Infinity, top: Infinity, right: -Infinity, bottom: -Infinity },
  );
  const pad = Math.max(width(union), height(union)) * FRAME_PADDING;
  let frame = {
    left: union.left - pad,
    top: union.top - pad,
    right: union.right + pad,
    bottom: union.bottom + pad,
  };
  const ratio = height(frame) / width(frame);
  if (ratio > MAX_ASPECT) {
    const extra = (height(frame) / MAX_ASPECT - width(frame)) / 2;
    frame = { ...frame, left: frame.left - extra, right: frame.right + extra };
  } else if (ratio < MIN_ASPECT) {
    const extra = (width(frame) * MIN_ASPECT - height(frame)) / 2;
    frame = { ...frame, top: frame.top - extra, bottom: frame.bottom + extra };
  }
  return frame;
}

export function sideOf(moving: Box, anchor: Box): Side {
  const cx = (moving.left + moving.right) / 2 - (anchor.left + anchor.right) / 2;
  const cy = (moving.top + moving.bottom) / 2 - (anchor.top + anchor.bottom) / 2;
  const nx = cx / ((width(anchor) + width(moving)) / 2);
  const ny = cy / ((height(anchor) + height(moving)) / 2);
  if (Math.abs(nx) >= Math.abs(ny)) return nx >= 0 ? "right" : "left";
  return ny >= 0 ? "below" : "above";
}

function along(start: number, size: number, anchorStart: number, anchorSize: number, fine: boolean): number {
  const minOverlap = Math.min(size, anchorSize) * MIN_OVERLAP_RATIO;
  const lo = anchorStart - size + minOverlap;
  const hi = anchorStart + anchorSize - minOverlap;
  let value = Math.min(Math.max(start, lo), hi);
  if (fine) return value;
  const threshold = Math.max(size, anchorSize) * SNAP_RATIO;
  const targets = [anchorStart, anchorStart + anchorSize - size, anchorStart + (anchorSize - size) / 2];
  for (const target of targets) {
    if (Math.abs(value - target) <= threshold) {
      value = target;
      break;
    }
  }
  return value;
}

export function attach(moving: LayoutDevice, pos: Point, anchor: LayoutDevice, anchorPos: Point, fine = false): Point {
  const a = deviceBox(anchor, anchorPos);
  const m = deviceBox(moving, pos);
  const chosen = sideOf(m, a);
  let left = m.left;
  let top = m.top;
  if (chosen === "left" || chosen === "right") {
    left = chosen === "right" ? a.right : a.left - width(m);
    top = along(m.top, height(m), a.top, height(a), fine);
  } else {
    top = chosen === "below" ? a.bottom : a.top - height(m);
    left = along(m.left, width(m), a.left, width(a), fine);
  }
  return { x: Math.round(pos.x + left - m.left), y: Math.round(pos.y + top - m.top) };
}

export function percent(value: number, start: number, size: number): string {
  return `${((value - start) / size) * 100}%`;
}
