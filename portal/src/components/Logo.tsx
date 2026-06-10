/** Bifrost snowflake mark — a 6-fold symmetric snowflake drawn in `currentColor`. */
export function Logo({ className }: { className?: string }) {
  const arm = (
    <>
      <line x1="0" y1="0" x2="0" y2="-19" />
      <line x1="0" y1="-13" x2="6" y2="-17" />
      <line x1="0" y1="-13" x2="-6" y2="-17" />
      <line x1="0" y1="-8" x2="4.5" y2="-11" />
      <line x1="0" y1="-8" x2="-4.5" y2="-11" />
    </>
  )
  return (
    <svg
      viewBox="0 0 48 48"
      className={className}
      fill="none"
      stroke="currentColor"
      strokeWidth={2.5}
      strokeLinecap="round"
      strokeLinejoin="round"
      role="img"
      aria-label="Bifrost"
    >
      <g transform="translate(24 24)">
        {[0, 60, 120, 180, 240, 300].map((a) => (
          <g key={a} transform={`rotate(${a})`}>
            {arm}
          </g>
        ))}
      </g>
    </svg>
  )
}
