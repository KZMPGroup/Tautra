import { useEffect, useState, type FormEvent } from "react";
import { Button } from "../../components/Button";
import { Card } from "../../components/Card";
import { ErrorMessage } from "../../components/Feedback";
import { Field, inputClass, Toggle } from "../../components/Field";
import { useI18n } from "../../i18n";
import { deviceNameProblem, MAX_DEVICE_NAME_LEN, useConfigSaver } from "../../lib/config";
import type { Config, Language } from "../../lib/types";

const LANGUAGES: { value: Language; label: string }[] = [
  { value: "id", label: "Bahasa Indonesia" },
  { value: "en", label: "English" },
];

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

        <div className="grid gap-2">
          <p className="text-sm font-medium text-neutral-900 dark:text-neutral-100">{t.general.language}</p>
          <div role="radiogroup" aria-label={t.general.language} className="inline-flex w-fit rounded-md bg-black/5 p-0.5 dark:bg-white/10">
            {LANGUAGES.map((lang) => {
              const active = config.general.language === lang.value;
              return (
                <button
                  key={lang.value}
                  type="button"
                  role="radio"
                  aria-checked={active}
                  disabled={saver.busy}
                  onClick={() =>
                    !active && void saver.save((c) => ({ ...c, general: { ...c.general, language: lang.value } }))
                  }
                  className={`rounded px-3 py-1 text-sm transition-colors focus-visible:outline-2 focus-visible:outline-indigo-500 ${
                    active
                      ? "bg-white font-medium text-neutral-900 shadow-sm dark:bg-neutral-700 dark:text-neutral-50"
                      : "text-neutral-600 hover:text-neutral-900 dark:text-neutral-300 dark:hover:text-neutral-50"
                  }`}
                >
                  {lang.label}
                </button>
              );
            })}
          </div>
        </div>

        <div className="border-t border-black/5 pt-4 dark:border-white/10">
          <Toggle
            label={t.general.autostart}
            hint={t.general.autostartHint}
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
