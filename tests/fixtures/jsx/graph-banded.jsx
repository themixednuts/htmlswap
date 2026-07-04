const SECTORS = [
  {
    rel: 'contains', label: 'Contains', count: 2, color: '#c45d4f',
    items: [
      { name: 'parser', kind: 'module' },
      { name: 'Query', kind: 'struct' },
    ],
  },
  {
    rel: 'uses', label: 'Used by', count: 1, color: '#6d7a5b',
    items: [
      { name: 'drizzle', kind: 'crate' },
    ],
  },
];

const KIND_FILL_C = {
  crate: 'var(--kind-crate)',
  module: 'var(--kind-module)',
  struct: 'var(--kind-struct)',
};

function GraphBanded() {
  const W = 420, H = 260;
  const cx = W / 2, cy = H / 2;
  const rInner = 40;
  const rOuter = 110;
  const startAngle = -90;
  const gap = 4;
  const total = 360 - gap * SECTORS.length;
  const perSector = total / SECTORS.length;
  const toRad = (d) => (d * Math.PI) / 180;
  const polar = (r, deg) => ({ x: cx + r * Math.cos(toRad(deg)), y: cy + r * Math.sin(toRad(deg)) });
  const arcPath = (r0, r1, a0, a1) => {
    const p0 = polar(r0, a0);
    const p1 = polar(r1, a0);
    const p2 = polar(r1, a1);
    const p3 = polar(r0, a1);
    const largeArc = a1 - a0 > 180 ? 1 : 0;
    return `M ${p0.x} ${p0.y} L ${p1.x} ${p1.y} A ${r1} ${r1} 0 ${largeArc} 1 ${p2.x} ${p2.y} L ${p3.x} ${p3.y} A ${r0} ${r0} 0 ${largeArc} 0 ${p0.x} ${p0.y} Z`;
  };

  return (
    <div className="relative" style={{ width: W, height: H, background: 'var(--panel)', borderRadius: 18, overflow: 'hidden' }}>
      <svg width={W} height={H} className="absolute inset-0">
        <g fill="none" stroke="var(--grid-line)" strokeWidth="1">
          {[60, 90].map(r => <circle key={r} cx={cx} cy={cy} r={r} />)}
        </g>
        {SECTORS.map((s, i) => {
          const a0 = startAngle + i * (perSector + gap);
          const a1 = a0 + perSector;
          const midA = (a0 + a1) / 2;
          const headerPos = polar(rOuter + 12, midA);
          const itemsTopRow = Math.ceil(s.items.length / 2);
          const itemsBottomRow = s.items.length - itemsTopRow;
          return (
            <g key={s.rel}>
              <path d={arcPath(rInner, rOuter, a0, a1)} fill={s.color} fillOpacity="0.06" stroke={s.color} strokeOpacity="0.25" strokeWidth="1" />
              <g transform={`translate(${headerPos.x}, ${headerPos.y})`}>
                <text textAnchor="middle" fill={s.color} fontFamily="var(--font-body)" fontSize="11" fontWeight="700" letterSpacing="0.16em" style={{ textTransform: 'uppercase' }}>
                  {s.label} / {s.count}
                </text>
              </g>
              {s.items.map((item, j) => {
                const row = j < itemsTopRow ? 0 : 1;
                const colCount = row === 0 ? itemsTopRow : itemsBottomRow;
                const colIdx = row === 0 ? j : j - itemsTopRow;
                const r = row === 0 ? rOuter - 24 : rInner + 20;
                const tStart = a0 + 8;
                const tEnd = a1 - 8;
                const t = colCount === 1 ? (tStart + tEnd) / 2 : tStart + (colIdx) * (tEnd - tStart) / (colCount - 1);
                const pos = polar(r, t);
                const fill = KIND_FILL_C[item.kind] || 'var(--muted)';
                return (
                  <g key={item.name} transform={`translate(${pos.x}, ${pos.y})`}>
                    <circle cx={0} cy={0} r="3.5" fill={fill} />
                    <text textAnchor="start" x={8} y="4" fill="var(--ink)" fontFamily="var(--font-body)" fontSize="12" fontWeight="600">
                      {item.name}
                    </text>
                  </g>
                );
              })}
            </g>
          );
        })}
      </svg>
    </div>
  );
}

window.GraphBanded = GraphBanded;
