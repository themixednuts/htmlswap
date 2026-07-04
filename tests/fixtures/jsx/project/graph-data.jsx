const NODES = {
  root: { kind: 'crate', path: 'demo' },
};

function nodeOf(id) {
  return { id, ...(NODES[id] || { kind: 'module', path: id }) };
}

Object.assign(window, { NODES, nodeOf });
