const minimumBpm = Number.parseFloat("{{{minimum_bpm}}}")

const bpmInput = document.getElementById("bpm")
const prettyPlease = document.getElementById("pretty-please")
const pleaseWait = document.getElementById("please-wait")
const tapTempo = document.getElementById("tap-tempo")
const results = document.getElementById("results")
let dance = document.getElementById("dance")
const permalink = document.getElementById("permalink")
const progress = document.getElementById("progress")
const errorText = document.getElementById("error-text")
const finalResultBox = document.getElementById("final-result")

function getLink(bpm) {
    return `${window.location.protocol}//{{{root}}}/${bpm}.gif`
}

let rendered = false
function loadSmugDance() {
    document.body.dataset.state = "loading"

    const bpm = bpmInput.value
    const link = getLink(bpm)
    permalink.href = link

    const img = new Image()
    dance.parentElement.replaceChild(img, dance)
    dance = img

    const xhr = new XMLHttpRequest()
    xhr.responseType = "arraybuffer"
    progress.dataset.progress = "indeterminate"
    progress.style.removeProperty("background-image")
    xhr.onprogress = (event) => {
        if (event.lengthComputable) {
            progress.dataset.progress = "specific"
            const percent = event.loaded / event.total * 100
            progress.style.backgroundImage = `
                linear-gradient(to right,
                    var(--progress-bar-fill) 0% ${percent}%,
                    var(--progress-bar-background) ${percent}% 100%)
            `
        }
    }
    xhr.onload = (_) => {
        if (xhr.status == 200) {
            const blob = new Blob([xhr.response])
            img.src = URL.createObjectURL(blob)
            img.id = "dance"
            img.onload = () => {
                document.body.dataset.state = "done"
                img.scrollIntoView({
                    behavior: "smooth",
                    block: "center",
                    inline: "center",
                })
            }
        } else {
            document.body.dataset.state = "error"
            const text = new TextDecoder("utf-8").decode(xhr.response)
            const err = JSON.parse(text)
            if (xhr.status == 500) {
                errorText.innerHTML = `
                    <em>${err.error}</em><br>
                    This is an internal server error, and it is not your fault.
                    Please report this, including the full error message, to
                    <a href="https://github.com/liquidev/smugdancer/issues">our issue tracker</a>.
                `
            } else {
                errorText.innerText = err.error
            }
        }
    }
    xhr.open("GET", link)
    xhr.send()
}

prettyPlease.onclick = loadSmugDance

const tapIndicators = Array.from(document.getElementsByClassName("tap"))
let tapTempoClickTimes = []
let tapTempoClickCount = -1
let resetTimer = null
tapTempo.onclick = () => {
    tapTempoClickTimes.push(performance.now())
    ++tapTempoClickCount
    if (tapTempoClickTimes.length > 32) {
        tapTempoClickTimes.splice(0, 1)
    }

    let averageDeltaMs = 0
    for (let i = 0; i < tapTempoClickTimes.length - 1; ++i) {
        const [first, second] = [tapTempoClickTimes[i], tapTempoClickTimes[i + 1]]
        const delta = second - first
        averageDeltaMs += delta
    }
    averageDeltaMs /= tapTempoClickTimes.length - 1

    if (tapTempoClickTimes.length > 4) {
        const averageBpm = 1000 / averageDeltaMs * 60
        bpmInput.value = Math.round(averageBpm).toString()
    }

    updateTapIndicators()

    clearTimeout(resetTimer)
    resetTimer = setTimeout(() => {
        tapTempoClickTimes.splice(0)
        tapTempoClickCount = -1
        updateTapIndicators()
    }, 2000)
}

function updateTapIndicators() {
    for (const i in tapIndicators) {
        const indicator = tapIndicators[i]
        const colorIndex = Math.ceil((tapTempoClickTimes.length - i) / tapIndicators.length)
        indicator.style.backgroundColor = `var(--tap-${colorIndex})`
    }

    if (tapTempoClickCount != -1) {
        const currentIndicator = tapIndicators[tapTempoClickCount % tapIndicators.length]
        currentIndicator.style.animation = ""
        void currentIndicator.offsetWidth; // Jesus fuck.
        currentIndicator.style.animation = "0.5s cubic-bezier(.07,.5,.25,1) beat"
    }
}

const bpmDrag = document.getElementById("bpm-drag")
let draggingBpm = false
let draggedBpm
bpmDrag.onmousedown = () => {
    bpmDrag.requestPointerLock()
}
bpmDrag.onmouseup = () => {
    document.exitPointerLock()
    draggingBpm = false
}
document.addEventListener("pointerlockchange", event => {
    if (document.pointerLockElement == bpmDrag) {
        draggingBpm = true
        draggedBpm = Number.parseFloat(bpmInput.value)
    }
})

document.addEventListener("mousemove", event => {
    if (draggingBpm) {
        draggedBpm += event.movementX / 10
        draggedBpm = Math.min(Math.max(draggedBpm, minimumBpm), 18000)
        bpmInput.value = Math.round(draggedBpm).toString()
    }
})
