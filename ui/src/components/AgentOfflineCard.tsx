import { useI18n } from "../i18n";
import { startAgent, useAction } from "../lib/agent";
import { Button } from "./Button";
import { Card } from "./Card";
import { ErrorMessage } from "./Feedback";

export function AgentOfflineCard({ error }: { error: string | null }) {
  const { t } = useI18n();
  const start = useAction();
  return (
    <Card title={t.home.agentOfflineTitle} description={t.home.agentOfflineBody}>
      <div className="grid gap-3">
        {error && <p className="selectable text-sm text-red-700 dark:text-red-300">{error}</p>}
        {start.error && <ErrorMessage error={start.error} onDismiss={start.clear} />}
        <div>
          <Button variant="primary" icon="play" busy={start.busy} onClick={() => void start.run(startAgent)}>
            {t.home.startAgent}
          </Button>
        </div>
      </div>
    </Card>
  );
}
