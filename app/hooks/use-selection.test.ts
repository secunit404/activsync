import { act, renderHook } from "@testing-library/react";
import { expect, test } from "vitest";

import { useSelection } from "./use-selection";

test("toggle adds then removes", () => {
  const { result } = renderHook(() => useSelection());
  act(() => result.current.toggle(1));
  expect(result.current.count).toBe(1);
  act(() => result.current.toggle(1));
  expect(result.current.count).toBe(0);
});

test("toggle does not mutate the previous set", () => {
  const { result } = renderHook(() => useSelection());
  const before = result.current.selected;
  act(() => result.current.toggle(1));
  expect(before.has(1)).toBe(false);
  expect(result.current.selected).not.toBe(before);
});

test("toggleAll selects all then clears when already full", () => {
  const { result } = renderHook(() => useSelection());
  act(() => result.current.toggleAll([1, 2, 3]));
  expect(result.current.count).toBe(3);
  act(() => result.current.toggleAll([1, 2, 3]));
  expect(result.current.count).toBe(0);
});

test("toggleAll replaces the selection with exactly the given ids", () => {
  const { result } = renderHook(() => useSelection());
  act(() => result.current.toggle(9));
  act(() => result.current.toggleAll([1, 2]));
  expect(Array.from(result.current.selected).sort()).toEqual([1, 2]);
});

test("clear empties the selection", () => {
  const { result } = renderHook(() => useSelection());
  act(() => result.current.toggle(1));
  act(() => result.current.toggle(2));
  act(() => result.current.clear());
  expect(result.current.count).toBe(0);
});
