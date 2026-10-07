(async () => {
    const t = document.getElementById("text");
    t.value = 'Meet me on <say-as type="date">12/25/2004</say-as>, <emphasis>please</emphasis>.';
    t.dispatchEvent(new Event("input"));
    document.getElementById("readMarkup").checked = true;
    document.getElementById("speakBtn").click();
    await new Promise((r) => setTimeout(r, 2500));
    const out = {
        speakButton: document.getElementById("speakBtn").textContent,
        stopEnabled: !document.getElementById("stopBtn").disabled,
        status: document.getElementById("status").textContent,
        statusIsError: document.getElementById("status").classList.contains("error"),
        voices: Array.from(document.getElementById("voice").options).map((o) => o.value),
    };
    document.getElementById("stopBtn").click();
    return out;
})()
