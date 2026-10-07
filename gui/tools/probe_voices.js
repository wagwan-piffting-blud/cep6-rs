(async () => {
    const list = await window.__TAURI__.core.invoke("voices", { root: null });
    return {
        names: list.names,
        root: list.root,
        selected: document.getElementById("voice").value,
        speakEnabled: !document.getElementById("speakBtn").disabled,
        status: document.getElementById("status").textContent,
    };
})()
