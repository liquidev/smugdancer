const bpmInput = document.getElementById("bpm")
const prettyPlease = document.getElementById("pretty-please")
const pleaseWait = document.getElementById("please-wait")
const tapTempo = document.getElementById("tap-tempo")
const results = document.getElementById("results")
let dance = document.getElementById("dance")
const permalink = document.getElementById("permalink")
const progress = document.getElementById("progress")
const errorResults = document.getElementById("error-results")
const error = document.getElementById("error")

function getLink(bpm) {
    return `${window.location.protocol}//{{{root}}}/${bpm}.gif`
}

function smugDanceLoaded() {
    pleaseWait.hidden = true
    results.hidden = false
}

function loadSmugDance() {
    results.hidden = true
    pleaseWait.hidden = false
    errorResults.hidden = true

    const bpm = bpmInput.value
    const link = getLink(bpm)

    permalink.innerText = link
    permalink.href = link

    const img = new Image()
    dance.parentElement.replaceChild(img, dance)
    dance = img

    const xhr = new XMLHttpRequest()
    xhr.responseType = "arraybuffer"
    progress.removeAttribute("value")
    xhr.onprogress = (event) => {
        if (event.lengthComputable) {
            progress.value = event.loaded / event.total
        }
    }
    xhr.onload = (_) => {
        if (xhr.status == 200) {
            const blob = new Blob([xhr.response])
            img.src = URL.createObjectURL(blob)
            img.onload = smugDanceLoaded
            img.id = "dance"
        } else {
            errorResults.hidden = false
            pleaseWait.hidden = true
            const text = new TextDecoder("utf-8").decode(xhr.response)
            const err = JSON.parse(text)
            if (xhr.status == 500) {
                error.innerHTML = `
                    I'm extremely sorry but an error occured on the server: <br>
                    <em>${err.error}</em><br>
                    This is not your fault. Please report this, including the full error message, at
                    <a href="https://github.com/liquidev/smugdancer/issues">the server's issue tracker</a>.
                `
            } else {
                error.innerText = err.error
            }
        }
    }
    xhr.open("GET", link)
    xhr.send()
}

prettyPlease.onclick = loadSmugDance
dance.onload = smugDanceLoaded

let tapTempoClickTimes = []
tapTempo.onclick = () => {
    tapTempoClickTimes.push(performance.now())
    if (tapTempoClickTimes.length > 32) {
        tapTempoClickTimes.splice(0, 1)
    }

    let averageDeltaMs = 0
    for (let i = 0; i < tapTempoClickTimes.length - 1; ++i) {
        const [first, second] = [tapTempoClickTimes[i], tapTempoClickTimes[i + 1]];
        const delta = second - first
        averageDeltaMs += delta
    }
    averageDeltaMs /= tapTempoClickTimes.length - 1

    const averageBpm = 1000 / averageDeltaMs * 60
    if (averageBpm < 60) {
        tapTempoClickTimes.splice(0);
    }

    if (tapTempoClickTimes.length > 4) {
        bpmInput.value = Math.round(averageBpm).toString()
    }
}
