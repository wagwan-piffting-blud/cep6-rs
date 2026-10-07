(async () => {
    const out = {};
    try {
        const wav = await window.__TAURI__.core.invoke("speak", { req: {
            text: "Hello there.", voice: document.getElementById("voice").value || "William",
            markup: false, rate: 0, pitch: 100, volume: 100 } });
        out.type = Object.prototype.toString.call(wav);
        out.length = wav.byteLength ?? wav.length;
        const bytes = new Uint8Array(wav instanceof ArrayBuffer ? wav : wav.buffer ?? []);
        out.head = Array.from(bytes.slice(0, 12));
        const url = URL.createObjectURL(new Blob([wav], { type: "audio/wav" }));
        out.url = url;
        const a = new Audio();
        out.canPlayWav = a.canPlayType("audio/wav");
        a.src = url;
        await new Promise((res) => {
            a.onloadedmetadata = () => { out.duration = a.duration; res(); };
            a.onerror = () => { out.mediaError = a.error && a.error.code + " " + a.error.message; res(); };
            setTimeout(res, 3000);
        });
        const meta = document.querySelector("meta[http-equiv]");
        out.csp = meta ? meta.content : "(header)";
    } catch (e) {
        out.error = String(e);
    }
    return out;
})()
