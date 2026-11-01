import { useEffect, useId, useState } from "react";
import { Button } from "../../components/Button";
import { ErrorMessage } from "../../components/Feedback";
import { Field, inputClass } from "../../components/Field";
import { Icon, Spinner } from "../../components/Icon";
import { PIN_LENGTH, PinInput } from "../../components/PinInput";
import { fmt, useI18n } from "../../i18n";
import { agentCall, Method, useAction } from "../../lib/agent";
import type { DiscoveredDevice } from "../../lib/types";

const BROWSE_REFRESH_MS = 60_000;
const ADDRESS_PATTERN = /^[A-Za-z0-9.\-]+(:\d{1,5})?$/;

type Target = { kind: "device"; device: DiscoveredDevice } | { kind: "address"; address: string };

interface PairJoinProps {
  discovered: DiscoveredDevice[];
  joiningName: string | null;
  onBack: () => void;
}

export function PairJoin({ discovered, joiningName, onBack }: PairJoinProps) {
  const { t } = useI18n();
  const browse = useAction();
  const join = useAction();
  const [selected, setSelected] = useState<Target | null>(null);
  const [pin, setPin] = useState("");
  const [manualOpen, setManualOpen] = useState(false);
  const [address, setAddress] = useState("");
  const [addressError, setAddressError] = useState<string | null>(null);
  const labelId = useId();

  useEffect(() => {
    const request = () => void browse.run(() => agentCall(Method.PairingBrowse));
    request();
    const id = window.setInterval(request, BROWSE_REFRESH_MS);
    return () => window.clearInterval(id);
  }, [browse.run]);

  const open = discovered.filter((d) => d.pairing_open);
  const others = discovered.filter((d) => !d.pairing_open && !d.trusted);
  const busy = join.busy || joiningName !== null;

  const targetName = selected
    ? selected.kind === "device"
      ? selected.device.device_name
      : selected.address
    : "";

  const submit = (value: string) => {
    if (!selected || value.length !== PIN_LENGTH || busy) return;
    const params =
      selected.kind === "device"
        ? { device_id: selected.device.device_id, pin: value }
        : { address: selected.address, pin: value };
    void join.run(() => agentCall(Method.PairingJoin, params)).then((ok) => {
      if (!ok) setPin("");
    });
  };

  const submitAddress = () => {
    const value = address.trim();
    if (!ADDRESS_PATTERN.test(value)) {
      setAddressError(t.devices.manualInvalid);
      return;
    }
    setAddressError(null);
    setSelected({ kind: "address", address: value });
    setPin("");
    join.clear();
  };

  const back = () => {
    void agentCall(Method.PairingCancel).catch((err) => console.error("pairing.cancel gagal", err));
    onBack();
  };

  if (selected) {
    return (
      <div className="grid gap-4">
        <p id={labelId} className="text-sm font-medium text-neutral-900 dark:text-neutral-100">
          {fmt(t.devices.joinPinLabel, { name: targetName })}
        </p>
        <PinInput
          value={pin}
          disabled={busy}
          autoFocus
          labelledBy={labelId}
          onChange={(v) => {
            setPin(v);
            if (join.error) join.clear();
          }}
          onComplete={submit}
        />
        {busy && (
          <p className="flex items-center gap-2 text-sm text-neutral-600 dark:text-neutral-400" aria-live="polite">
            <Spinner />
            {fmt(t.devices.joinBusy, { name: joiningName ?? targetName })}
          </p>
        )}
        {join.error && <ErrorMessage error={join.error} onDismiss={join.clear} />}
        <div className="flex flex-wrap gap-2">
          <Button
            variant="primary"
            icon="key"
            busy={busy}
            disabled={pin.length !== PIN_LENGTH}
            onClick={() => submit(pin)}
          >
            {t.devices.joinSubmit}
          </Button>
          <Button
            icon="arrowLeft"
            disabled={busy}
            onClick={() => {
              setSelected(null);
              setPin("");
              join.clear();
            }}
          >
            {t.devices.joinBack}
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className="grid gap-4">
      {browse.error && <ErrorMessage error={browse.error} onDismiss={browse.clear} />}
      {open.length === 0 ? (
        <div className="grid gap-2">
          <p className="flex items-center gap-2 text-sm text-neutral-700 dark:text-neutral-300" aria-live="polite">
            <Spinner />
            {t.devices.joinSearching}
          </p>
          <p className="text-sm text-neutral-600 dark:text-neutral-400">{t.devices.joinNone}</p>
        </div>
      ) : (
        <ul className="grid gap-2">
          {open.map((d) => (
            <li key={d.device_id}>
              <button
                type="button"
                onClick={() => {
                  setSelected({ kind: "device", device: d });
                  setPin("");
                  join.clear();
                }}
                className="flex w-full items-center gap-3 rounded-lg bg-white/80 px-3.5 py-3 text-start ring-1 ring-black/10 transition-colors hover:bg-white focus-visible:outline-2 focus-visible:outline-indigo-500 dark:bg-white/5 dark:ring-white/10 dark:hover:bg-white/10"
              >
                <Icon name="devices" className="size-5 text-indigo-600 dark:text-indigo-400" />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-sm font-medium text-neutral-900 dark:text-neutral-100">
                    {d.device_name}
                  </span>
                  <span className="block truncate text-xs text-neutral-500 dark:text-neutral-400">
                    {d.addresses.join(", ")}
                  </span>
                </span>
                <Icon name="key" className="size-4 text-neutral-400" />
              </button>
            </li>
          ))}
        </ul>
      )}

      {others.length > 0 && (
        <div className="grid gap-1.5">
          <p className="text-xs font-medium text-neutral-500 dark:text-neutral-400">{t.devices.joinOthers}</p>
          <ul className="grid gap-1">
            {others.map((d) => (
              <li key={d.device_id} className="truncate text-sm text-neutral-600 dark:text-neutral-400">
                {d.device_name}
              </li>
            ))}
          </ul>
        </div>
      )}

      {manualOpen ? (
        <form
          className="grid gap-3 rounded-lg bg-white/60 p-3.5 ring-1 ring-black/10 dark:bg-white/5 dark:ring-white/10"
          onSubmit={(e) => {
            e.preventDefault();
            submitAddress();
          }}
        >
          <p className="text-sm font-medium text-neutral-900 dark:text-neutral-100">{t.devices.manualTitle}</p>
          <Field label={t.devices.manualLabel} hint={t.devices.manualHint} error={addressError}>
            {(id, describedBy) => (
              <input
                id={id}
                aria-describedby={describedBy}
                className={`${inputClass} font-mono`}
                value={address}
                autoFocus
                spellCheck={false}
                autoComplete="off"
                placeholder={t.devices.manualPlaceholder}
                onChange={(e) => {
                  setAddress(e.target.value);
                  if (addressError) setAddressError(null);
                }}
              />
            )}
          </Field>
          <div>
            <Button type="submit" variant="primary" icon="key" disabled={address.trim().length === 0}>
              {t.devices.manualNext}
            </Button>
          </div>
        </form>
      ) : null}

      <div className="flex flex-wrap gap-2">
        <Button icon="arrowLeft" onClick={back}>
          {t.common.cancel}
        </Button>
        {!manualOpen && (
          <Button icon="key" onClick={() => setManualOpen(true)}>
            {t.devices.manualOpen}
          </Button>
        )}
      </div>
    </div>
  );
}
