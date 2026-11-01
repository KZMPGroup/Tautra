import { useEffect, useState, type FormEvent } from "react";
import { Button } from "../../components/Button";
import { Card } from "../../components/Card";
import { ErrorMessage } from "../../components/Feedback";
import { Field, inputClass, Toggle } from "../../components/Field";
import { useI18n } from "../../i18n";
import { deviceNameProblem, MAX_DEVICE_NAME_LEN, useConfigSaver } from "../../lib/config";
import type { Config } from "../../lib/types";
import { LanguagePicker } from "./LanguagePicker";

export function DeviceSection({ config, effectiveName }: { config: Config; effectiveName: string }) {
  const { t } = useI18n();
  const saver = useConfigSaver();
  const saved = config.general.device_name;
  const [name, setName] = useState(saved);

  useEffect(() => setName(saved), [saved]);

  const problem = deviceNameProblem(name);
  const problemText =
    problem === "too_long" ? t.general.deviceNameTooLong : problem === "control" ? t.general.deviceNameControl : null;
  const dirty = name.trim() !== saved.trim();

  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (!dirty || problem) return;
    void saver.save((c) => ({ ...c, general: { ...c.general, device_name: name.trim() } }));
  };

  return (
    <Card title={t.general.deviceSection}>
      <div className="grid gap-5">
        <form onSubmit={submit} className="grid gap-2">
          <Field label={t.general.deviceName} hint={t.general.deviceNameHint} error={problemText}>
            {(id, describedBy) => (
              <div className="flex gap-2">
                <input
                  id={id}
                  value={name}
                  placeholder={effectiveName}
                  maxLength={MAX_DEVICE_NAME_LEN + 8}
                  aria-describedby={describedBy}
                  aria-invalid={problem ? true : undefined}
                  onChange={(e) => setName(e.currentTarget.value)}
                  className={`${inputClass} max-w-md`}
                />
                {dirty && (
                  <Button type="submit" variant="primary" busy={saver.busy} disabled={problem !== null}>
                    {t.common.save}
                  </Button>
                )}
              </div>
            )}
          </Field>
        </form>

        <LanguagePicker
          value={config.general.language}
          disabled={saver.busy}
          onChange={(language) => void saver.save((c) => ({ ...c, general: { ...c.general, language } }))}
        />

        <div className="border-t border-black/5 pt-4 dark:border-white/10">
          <Toggle
            label={t.general.autostart}
            hint={`${t.general.autostartHint} ${t.general.autostartLogonHint}`}
            checked={config.general.autostart}
            disabled={saver.busy}
            onChange={(autostart) => void saver.save((c) => ({ ...c, general: { ...c.general, autostart } }))}
          />
        </div>

        {saver.error && <ErrorMessage error={saver.error} onDismiss={saver.clear} />}
      </div>
    </Card>
  );
}
