import { KeyRoundIcon } from "lucide-react";

import { cn } from "@/lib/utils";

export function OidcLogo({ url, className }: { url: string | null; className?: string }) {
  if (url) {
    return (
      <img
        src={url}
        alt=""
        className={cn("h-5 w-auto max-w-20 object-contain", className)}
      />
    );
  }
  return <KeyRoundIcon className={cn("size-5", className)} aria-hidden="true" />;
}
