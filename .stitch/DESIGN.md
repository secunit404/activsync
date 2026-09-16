# Design System: ActivSync

**Project ID:** 11996681702707756995

## 1. Visual Theme & Atmosphere

Utilitarian, calm, and data-forward — a self-hosted developer/operations
dashboard, not a consumer marketing app. The mood is **restrained and airy**:
generous vertical rhythm, plenty of quiet whitespace, and a near-flat surface
treatment where structure comes from thin hairline borders rather than heavy
shadow. Numbers and identifiers are treated as first-class data (monospace),
while prose and labels are clean humanist sans-serif. Nothing is loud; the one
warm accent (a burnt orange) is reserved for the primary action so it always
stands out against an otherwise cool, neutral grey-blue field. Supports both a
light and a dark theme with equal care.

## 2. Color Palette & Roles

**Light theme (default):**
- **Cool Porcelain Grey-Blue (#f5f7f9)** — App background / page canvas.
- **Pure White (#ffffff)** — Card and panel surface; input fields.
- **Faint Mist Grey (#f0f3f6)** — Secondary raised surface (subtle zebra, wells).
- **Ink Slate (#18212b)** — Primary text and headings.
- **Muted Steel (#5c6976)** — Secondary/hint text, helper copy.
- **Hairline Grey (#d8e0e7)** — Default borders and dividers.
- **Field Stroke Grey (#c8d1db)** — Input outlines.
- **Burnt Sync Orange (#e85b2a)** — PRIMARY action color (save, confirm, CTA).
- **Deep Ember (#c5441b)** — Primary hover / pressed accent ink.
- **Garmin Steel Blue (#176ca5)** — Garmin-branded actions and links.

**Semantic status colors** (used for pills/badges, always paired bg + ink):
- **Success Green (#18834d)** on tint **#e2f3e9** — "Mapped" / healthy state.
- **Info Blue (#2865b5)** on tint **#e7f0fc** — In-flight / pending / suggested.
- **Caution Amber (#a56b08)** on tint **#fff3d6** — "Unmapped" / needs attention.
- **Alert Red (#b8392a)** on tint **#fde8e4** — "Rejected by Garmin" / error.
- **Neutral Grey (#66727d)** on tint **#eef1f4** — Built-in / inert / disabled.

**Dark theme** mirrors these roles on deep slate surfaces (bg #151a21, surface
#1c232b, ink #f3f6f8, muted #a7b2bd), with the accent brightening to a warmer
orange (#f06432) and status colors lifting to higher-luminance variants
(green #58d395, blue #75aafa, amber #e5b04c, red #ff806d).

## 3. Typography Rules

- **Primary family:** Inter (humanist sans-serif) for all headings, labels,
  buttons, and body copy. Clean, neutral, highly legible at small sizes.
- **Data family:** Monospace (SF Mono / JetBrains Mono / Menlo) for numbers,
  IDs, timestamps, counts, and technical identifiers — reinforces the
  "dashboard for real data" character and keeps figures aligned.
- **Weight usage:** Headings are semibold-to-bold and tight; body is regular;
  small helper text sits in Muted Steel at a slightly reduced size.
- **Letter-spacing:** Near-neutral; status pills use a small uppercase label
  with modest tracking to read as chips rather than prose.

## 4. Component Stylings

- **Buttons:** Subtly rounded corners (**6px**). Primary buttons fill with Burnt
  Sync Orange and white ink; secondary buttons are quiet — transparent or faint
  grey fill with a hairline border and Ink Slate text. Hover deepens the fill
  by a step. Flat by default; no gradients.
- **Cards / Panels / Containers:** White surface, **6px** corners, a single
  hairline (#d8e0e7) border, and a whisper-soft shadow (0 1px 2px at ~6%
  opacity) — depth is implied, never dramatic.
- **Inputs / Selects / Comboboxes:** White field, **4px** corners, a thin
  Field Stroke Grey outline that strengthens to the accent on focus with a soft
  focus ring. Comfortable padding; labels sit above the field in Muted Steel.
- **Status pills / Badges:** Small, **pill-shaped**, semantic tint background
  with matching darker ink and a tiny leading status dot.
- **Tables:** Quiet hairline row dividers, roomy row height, left-aligned text,
  monospace for any numeric or ID column, sticky header row.

## 5. Layout Principles

Single, comfortably-wide content column of stacked panels rather than a dense
multi-pane grid. Generous whitespace and consistent vertical rhythm between
sections. Alignment is strictly left-edge; related controls group into tidy
form-grids of two columns on wider viewports and collapse to one column when
narrow. Hierarchy is created through spacing, weight, and the single warm
accent — not through borders or color noise. Desktop-first, but fluid.
