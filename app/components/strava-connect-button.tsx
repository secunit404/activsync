import connectWithStrava from "@/assets/btn_strava_connect_with_orange.svg";
import { cn } from "@/lib/utils";

/**
 * Strava's official "Connect with Strava" button, which its brand guidelines
 * require, unmodified, on every Strava OAuth entry point. It submits its form,
 * so saving the credentials and leaving for Strava is a single step.
 */
export function StravaConnectButton({
  form,
  disabled,
  pending,
  error,
  className,
}: {
  form?: string;
  disabled?: boolean;
  pending?: boolean;
  error?: string | null;
  className?: string;
}) {
  return (
    <div className={cn("grid w-fit gap-2", className)}>
      <button
        type="submit"
        form={form}
        disabled={disabled || pending}
        aria-busy={pending || undefined}
        className="inline-flex w-fit rounded-md outline-none focus-visible:ring-3 focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50"
      >
        <img src={connectWithStrava} alt="Connect with Strava" width={237} height={48} />
      </button>
      {error ? (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      ) : null}
    </div>
  );
}
