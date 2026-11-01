import { useEffect, useState, type ReactNode } from "react";
import { Button } from "../../components/Button";
import { Card } from "../../components/Card";
import { ErrorMessage, SuccessMessage } from "../../components/Feedback";
import { Icon, type IconName } from "../../components/Icon";
import { fmt, useI18n } from "../../i18n";
import { agentCall, Method, useAction } from "../../lib/agent";
import type { AgentStatus, PairingHostResult } from "../../lib/types";
import { PairHost } from "./PairHost";
import { PairJoin } from "./PairJoin";

type Mode = "choose" | "join";

export function PairCard({ status }: { status: AgentStatus }) {
  const { t } = useI18n();
  const [mode, setMode] = useState<Mode>("choose");
  const host = useAction();
  const pairing = status.pairing;

  useEffect(() => {
    if (pairing.state === "succeeded") setMode("choose");
  }, [pairing.state]);

  let body;
  if (pairing.state === "hosting") {
    body = (
      <PairHost
        pin={pairing.pin}
        expiresUnixMs={pairing.expires_unix_ms}
        attemptsLeft={pairing.attempts_left}
        addresses={status.network.addresses.map((a) => `${a}:${status.network.quic_port}`)}
      />
    );
  } else if (mode === "join" || pairing.state === "joining") {
    body = (
      <PairJoin
        discovered={status.discovered}
        joiningName={pairing.state === "joining" ? pairing.device_name : null}
        onBack={() => setMode("choose")}
      />
    );
  } else {
    body = (
      <div className="grid gap-4">
        {pairing.state === "succeeded" && (
          <SuccessMessage text={fmt(t.devices.succeeded, { name: pairing.device_name })} />
        )}
        {pairing.state === "failed" && <ErrorMessage error={{ code: pairing.code, message: pairing.message }} />}
        {host.error && <ErrorMessage error={host.error} onDismiss={host.clear} />}
        <div className="grid gap-3 md:grid-cols-2">
          <Choice
            icon="key"
            title={t.devices.hostTitle}
            body={t.devices.hostBody}
            action={
              <Button
                variant="primary"
                busy={host.busy}
                onClick={() => void host.run(() => agentCall<PairingHostResult>(Method.PairingHost))}
              >
                {t.devices.hostStart}
              </Button>
            }
          />
          <Choice
            icon="search"
            title={t.devices.joinTitle}
            body={t.devices.joinBody}
            action={
              <Button
                onClick={() => {
                  host.clear();
                  setMode("join");
                }}
              >
                {t.devices.joinStart}
              </Button>
            }
          />
        </div>
      </div>
    );
  }

  return (
    <Card title={t.devices.pairTitle} description={t.devices.pairIntro}>
      {body}
    </Card>
  );
}

interface ChoiceProps {
  icon: IconName;
  title: string;
  body: string;
  action: ReactNode;
}

function Choice({ icon, title, body, action }: ChoiceProps) {
  return (
    <div className="flex flex-col gap-3 rounded-lg bg-black/3 p-4 dark:bg-white/5">
      <div className="flex items-center gap-2">
        <Icon name={icon} className="size-4.5 text-indigo-600 dark:text-indigo-400" />
        <h3 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">{title}</h3>
      </div>
      <p className="flex-1 text-sm text-neutral-600 dark:text-neutral-400">{body}</p>
      <div>{action}</div>
    </div>
  );
}
