function View() {
  const node = window.nodeOf('root');
  return (
    <main>
      <window.Badge label={node.path} />
      <Icon.search size={16} />
      <window.KindIcon kind={node.kind} />
    </main>
  );
}

window.View = View;
