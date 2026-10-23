import { ShieldOff } from "lucide-react";

export function AuthDisabledBanner() {
  return (
    <div
      role="status"
      className="flex items-center justify-center gap-2 border-b border-warning/30 bg-warning/10 px-4 py-2 text-center text-[13px] text-warning"
    >
      <ShieldOff className="size-4 shrink-0" aria-hidden="true" />
      Authentication is disabled. Anyone who can reach ActivSync can use it.
    </div>
  );
}
