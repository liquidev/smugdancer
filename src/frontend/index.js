const bpmInput = document.getElementById("bpm")
const prettyPlease = document.getElementById("pretty-please")
const pleaseWait = document.getElementById("please-wait")
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
    // img.src = link
    dance.parentElement.replaceChild(img, dance)
    dance = img

    const xhr = new XMLHttpRequest()
    xhr.responseType = "arraybuffer"
    progress.removeAttribute("value")
    xhr.onprogress = (event) => {
        console.log(event)
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
            error.innerText = err.error
        }
    }
    xhr.open("GET", link)
    xhr.send()
}

prettyPlease.onclick = loadSmugDance
dance.onload = smugDanceLoaded
