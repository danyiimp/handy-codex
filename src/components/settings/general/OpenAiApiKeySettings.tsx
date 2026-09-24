import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../../hooks/useSettings";
import { Button } from "../../ui/Button";
import { Input } from "../../ui/Input";
import { SettingContainer } from "../../ui/SettingContainer";

export const OpenAiApiKeySettings: React.FC = React.memo(() => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const storedKey = getSetting("openai_api_key") ?? "";
  const [draft, setDraft] = useState(storedKey);
  const [revealed, setRevealed] = useState(false);

  // A rejected update rolls the store back; follow it so the input never
  // shows a key the backend refused to persist.
  useEffect(() => {
    setDraft(storedKey);
  }, [storedKey]);

  const saveKey = async () => {
    const trimmed = draft.trim();
    if (!trimmed) {
      await updateSetting("openai_api_key", null);
      return;
    }
    await updateSetting("openai_api_key", trimmed);
  };

  const clearKey = async () => {
    setDraft("");
    await updateSetting("openai_api_key", null);
  };

  const dirty = draft.trim() !== storedKey.trim();

  return (
    <SettingContainer
      title={t("settings.general.openaiApiKey.title")}
      description={t("settings.general.openaiApiKey.description")}
    >
      <div className="flex flex-wrap items-center gap-2">
        <Input
          type={revealed ? "text" : "password"}
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={() => {
            if (!dirty) return;
            void saveKey();
          }}
          placeholder={t("settings.general.openaiApiKey.placeholder")}
          className="min-w-64 flex-1"
          variant="compact"
          aria-label={t("settings.general.openaiApiKey.title")}
        />
        <Button
          onClick={() => setRevealed((value) => !value)}
          variant="secondary"
          size="md"
          aria-label={t("settings.general.openaiApiKey.toggleVisibility")}
        >
          {revealed
            ? t("settings.general.openaiApiKey.hide")
            : t("settings.general.openaiApiKey.show")}
        </Button>
        {dirty && (
          <Button
            onClick={() => void saveKey()}
            disabled={isUpdating("openai_api_key")}
            variant="secondary"
            size="md"
          >
            {t("settings.general.openaiApiKey.save")}
          </Button>
        )}
        {storedKey && (
          <Button
            onClick={clearKey}
            disabled={isUpdating("openai_api_key")}
            variant="ghost"
            size="md"
          >
            {t("settings.general.openaiApiKey.clear")}
          </Button>
        )}
      </div>
    </SettingContainer>
  );
});
