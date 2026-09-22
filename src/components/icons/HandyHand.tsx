const HandyHand = ({
  width = 126,
  height = 126,
  className,
}: {
  width?: number | string;
  height?: number | string;
  className?: string;
}) => (
  <svg
    width={width}
    height={height}
    viewBox="0 0 64 64"
    className={className}
    fill="none"
    stroke="currentColor"
    strokeWidth="6"
    strokeLinecap="round"
    aria-hidden="true"
    xmlns="http://www.w3.org/2000/svg"
  >
    <path d="M8 29v6m12-17v28m12-35v42m12-32v22m12-16v10" />
  </svg>
);

export default HandyHand;
