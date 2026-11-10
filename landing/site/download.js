// The download page's only behaviour: copy the node command. Everything else is a link.
(() => {
  const button = document.querySelector(".copy");
  if (!button) return;

  const original = button.textContent;
  let timer = 0;

  const flash = (text) => {
    button.textContent = text;
    button.dataset.done = "true";
    window.clearTimeout(timer);
    timer = window.setTimeout(() => {
      button.textContent = original;
      delete button.dataset.done;
    }, 1600);
  };

  button.addEventListener("click", async () => {
    const command = button.dataset.copy || "";
    try {
      await navigator.clipboard.writeText(command);
      flash("COPIED");
    } catch {
      // Clipboard access can be refused (older browsers, denied permission). Select the
      // command instead so it is one keystroke away.
      const code = document.getElementById("node-command");
      const selection = window.getSelection();
      if (code && selection) {
        const range = document.createRange();
        range.selectNodeContents(code);
        selection.removeAllRanges();
        selection.addRange(range);
      }
      flash("SELECTED");
    }
  });
})();
