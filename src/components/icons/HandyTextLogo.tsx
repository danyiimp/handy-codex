import { useTranslation } from "react-i18next";

const HandyTextLogo = ({
  width,
  height,
  className,
}: {
  width?: number;
  height?: number;
  className?: string;
}) => {
  const { t } = useTranslation();

  return (
    <svg
      width={width}
      height={height}
      className={className}
      viewBox="0 0 130 60"
      role="img"
      aria-label={t("branding.appName")}
      xmlns="http://www.w3.org/2000/svg"
    >
      <text
        x="0"
        y="43"
        fontFamily="system-ui, sans-serif"
        fontSize="38"
        fontWeight="700"
        className="fill-text"
      >
        {t("branding.appName")}
      </text>
    </svg>
  );
};

export default HandyTextLogo;
