import connectWithStrava from "@/assets/btn_strava_connect_with_orange.svg";
import { cn } from "@/lib/utils";

/**
 * Strava's official "Connect with Strava" button, which its brand guidelines
 * require, unmodified, on every Strava OAuth entry point. Without an `href`
 * it renders disabled rather than linking anywhere else.
 */
export function StravaConnectButton({
  href,
  error,
  className,
}: {
  href: string | null;
  error?: string | null;
  className?: string;
}) {
  const image = (
    <img src={connectWithStrava} alt="Connect with Strava" width={237} height={48} />
  );
  return (
    <div className={cn("grid w-fit gap-2", className)}>
      {href ? (
        <a
          href={href}
          className="inline-flex w-fit rounded-md outline-none focus-visible:ring-3 focus-visible:ring-ring/50"
        >
          {image}
        </a>
      ) : (
        <span aria-disabled="true" className="inline-flex w-fit cursor-not-allowed opacity-50">
          {image}
        </span>
      )}
      {error ? (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      ) : null}
    </div>
  );
}
