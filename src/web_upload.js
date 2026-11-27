const pendingButtons = new Map();

function setPending(button, message) {
    pendingButtons.set(button, button.textContent);
    button.disabled = true;
    button.setAttribute("aria-busy", "true");
    button.textContent = message;
}

function clearPending(button) {
    button.textContent = pendingButtons.get(button);
    button.disabled = false;
    button.removeAttribute("aria-busy");
    pendingButtons.delete(button);
}

document.addEventListener("submit", (event) => {
    const button = event.submitter;
    if (event.defaultPrevented || !button?.dataset.pending) return;
    setPending(button, button.dataset.pending);
});

window.addEventListener("pageshow", () => pendingButtons.forEach((_, button) => clearPending(button)));

const upload = document.getElementById("upload");
upload.addEventListener("submit", async (event) => {
    event.preventDefault();
    const file = document.getElementById("torrent").files[0];
    const status = document.getElementById("upload-result");
    const button = event.submitter || upload.querySelector("button");
    if (!file || button.disabled) return;
    if (file.size > 10000000) {
        status.textContent = "Choose a torrent file under 10 MB.";
        return;
    }
    setPending(button, "Adding file…");
    status.textContent = "Adding file…";
    try {
        const result = await fetch("/torrent", {
            method: "POST",
            headers: { "Content-Type": "application/x-bittorrent" },
            body: file,
        });
        if (result.redirected && result.ok) {
            window.location.assign("/");
            return;
        }
        const error = new DOMParser().parseFromString(await result.text(), "text/html");
        status.textContent = (error.querySelector("p") || error.body).textContent;
    } catch {
        status.textContent = "Could not reach Fero.";
    }
    clearPending(button);
});
