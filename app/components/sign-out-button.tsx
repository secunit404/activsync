import { useMutation } from "@tanstack/react-query";
import { LogOut } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { logout, type AuthMethod } from "@/lib/auth-api";

/**
 * Hidden for proxy sign-ins (the proxy owns that session, so signing out
 * here would not sign anyone out) and when auth is disabled.
 *
 * Lands on `?local=1` so an auto-redirecting single sign-on provider with a
 * live session does not sign the user straight back in.
 */
export function SignOutButton({
  method,
  variant = "icon",
}: {
  method: AuthMethod | null;
  variant?: "icon" | "text";
}) {
  const mutation = useMutation({
    mutationFn: logout,
    onSuccess: () => window.location.assign("/login?local=1"),
    onError: (error) => toast.error(error.message),
  });
  if (method === null || method === "proxy" || method === "disabled") {
    return null;
  }
  if (variant === "text") {
    return (
      <Button
        type="button"
        variant="outline"
        disabled={mutation.isPending}
        onClick={() => mutation.mutate()}
      >
        <LogOut aria-hidden="true" /> Sign out
      </Button>
    );
  }
  return (
    <Button
      type="button"
      variant="ghost"
      size="icon"
      aria-label="Sign out"
      title="Sign out"
      className="size-[42px] text-muted-foreground"
      disabled={mutation.isPending}
      onClick={() => mutation.mutate()}
    >
      <LogOut className="size-[19px]" aria-hidden="true" />
    </Button>
  );
}
