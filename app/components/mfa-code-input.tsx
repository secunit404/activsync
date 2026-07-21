import { useRef, useState, type ClipboardEvent, type KeyboardEvent } from "react";

import { cn } from "@/lib/utils";

const DIGIT_PATTERN = /^[0-9]$/;

export type MfaCodeInputProps = {
  /** Number of digit boxes to render. */
  length: number;
  /** Fires exactly once, with the full code, the moment every box is filled. */
  onComplete: (code: string) => void;
  disabled?: boolean;
};

/**
 * A row of single-character digit boxes for a Garmin MFA code (frame `3c`).
 * Auto-advances on entry, Backspace on an already-empty box steps focus back
 * a box, and pasting (or an OS one-time-code autofill, which lands as a
 * multi-character `onChange` rather than a paste event) distributes the
 * whole code across every box in one pass. Non-numeric input is rejected —
 * the box is reset to its previous value rather than showing the rejected
 * character.
 */
export function MfaCodeInput({ length, onComplete, disabled = false }: MfaCodeInputProps) {
  const [digits, setDigits] = useState<string[]>(() => Array(length).fill(""));
  const boxRefs = useRef<Array<HTMLInputElement | null>>([]);

  function focusBox(index: number) {
    boxRefs.current[index]?.focus();
  }

  function commit(next: string[]) {
    setDigits(next);
    if (next.every((digit) => digit !== "")) {
      onComplete(next.join(""));
    }
  }

  function distribute(startIndex: number, raw: string) {
    const clean = raw.replace(/\D/g, "");
    if (!clean) {
      return;
    }
    const next = [...digits];
    let cursor = startIndex;
    for (const char of clean) {
      if (cursor >= length) {
        break;
      }
      next[cursor] = char;
      cursor += 1;
    }
    commit(next);
    focusBox(Math.min(cursor, length - 1));
  }

  function handleChange(index: number, rawValue: string) {
    if (rawValue.length > 1) {
      // A whole code landed at once — OS one-time-code autofill, most
      // commonly, since browsers often bypass `maxLength` for it.
      distribute(index, rawValue);
      return;
    }
    if (rawValue && !DIGIT_PATTERN.test(rawValue)) {
      // Reject silently: re-render with the unchanged array so React resets
      // the controlled value and discards whatever the browser just typed.
      setDigits((previous) => [...previous]);
      return;
    }
    const next = [...digits];
    next[index] = rawValue;
    commit(next);
    if (rawValue && index < length - 1) {
      focusBox(index + 1);
    }
  }

  function handleKeyDown(index: number, event: KeyboardEvent<HTMLInputElement>) {
    if (event.key === "Backspace" && digits[index] === "" && index > 0) {
      event.preventDefault();
      const next = [...digits];
      next[index - 1] = "";
      setDigits(next);
      focusBox(index - 1);
    }
  }

  function handlePaste(event: ClipboardEvent<HTMLInputElement>) {
    event.preventDefault();
    distribute(0, event.clipboardData.getData("text"));
  }

  return (
    <div className="flex gap-2">
      {digits.map((digit, index) => (
        <input
          key={index}
          ref={(element) => {
            boxRefs.current[index] = element;
          }}
          aria-label={`Digit ${index + 1}`}
          className={cn(
            "h-[52px] w-0 min-w-0 flex-1 rounded-[10px] border border-input bg-[#0b0f14] text-center font-mono text-xl text-foreground outline-none transition-colors",
            "focus:border-primary focus:ring-4 focus:ring-primary/10",
          )}
          inputMode="numeric"
          autoComplete={index === 0 ? "one-time-code" : "off"}
          maxLength={1}
          value={digit}
          disabled={disabled}
          onChange={(event) => handleChange(index, event.target.value)}
          onKeyDown={(event) => handleKeyDown(index, event)}
          onPaste={handlePaste}
        />
      ))}
    </div>
  );
}
