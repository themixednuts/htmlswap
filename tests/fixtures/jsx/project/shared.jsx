function Badge({ label }) {
  return <span className="badge">{label}</span>;
}

const Icon = {
  search: (p = {}) => <svg width={p.size || 12} height={p.size || 12} viewBox="0 0 24 24" {...p}><path d="M0 0h10" /></svg>,
};

function KindBadge({ kind }) {
  return <span>{kind}</span>;
}

Object.assign(window, { Badge, Icon, KindBadge });
if (!('KindIcon' in window) && 'KindBadge' in window) window.KindIcon = window.KindBadge;
