import { CircleCheck, CircleX } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";

type ConnectionStatusProps = {
  name: string;
  connected: boolean;
  status: string;
  detail?: string;
};

export function ConnectionStatus({
  name,
  connected,
  status,
  detail,
}: ConnectionStatusProps) {
  const Icon = connected ? CircleCheck : CircleX;

  return (
    <Card className="gap-4 py-5 shadow-sm">
      <CardHeader className="flex-row items-start justify-between gap-3 px-5">
        <div className="grid gap-1">
          <CardTitle className="text-base">{name}</CardTitle>
          <CardDescription>{status}</CardDescription>
        </div>
        <Badge variant={connected ? "secondary" : "destructive"}>
          <Icon data-icon="inline-start" />
          {connected ? "Ready" : "Paused"}
        </Badge>
      </CardHeader>
      {detail ? (
        <CardContent className="px-5 text-sm text-muted-foreground">
          {detail}
        </CardContent>
      ) : null}
    </Card>
  );
}
