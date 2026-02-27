/* Fix: Prevent "Copy code" button label from appearing in clipboard content.
 *
 * sphinxawesome_theme inserts a copy button inside <pre> elements. When
 * clipboard.js selects all children of the <pre> to copy, the button's
 * sr-only text ("Copy code") is included in the selection. This listener
 * intercepts the copy event and strips that trailing label from the data
 * written to the clipboard.
 */
document.addEventListener('copy', function (e) {
    if (!e.clipboardData) { return; }
    var selection = window.getSelection();
    if (!selection) { return; }
    var text = selection.toString();
    var clean = text.replace(/\nCopy code\s*$/, '');
    if (clean === text) { return; }
    e.clipboardData.setData('text/plain', clean);
    e.preventDefault();
}, true);
