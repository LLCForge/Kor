// Kor OCR on macOS: the Vision framework built into macOS 10.15+ (VNRecognizeTextRequest), run by
// `osascript -l JavaScript`. Inputs only from the environment: KOR_OCR_PATH, KOR_OCR_LANG.
// Prints {"lang", "lines": [{text, x, y, w, h}]} in image pixels, top-left origin, like ocr.ps1.
ObjC.import('Foundation');
ObjC.import('AppKit');
ObjC.import('Vision');

function run() {
  const env = ObjC.deepUnwrap($.NSProcessInfo.processInfo.environment);
  const path = String(env.KOR_OCR_PATH || '');
  const lang = String(env.KOR_OCR_LANG || '');
  const rep = $.NSBitmapImageRep.imageRepWithContentsOfFile(path);
  if (rep.isNil()) throw new Error('The image could not be decoded.');
  const width = rep.pixelsWide;
  const height = rep.pixelsHigh;
  const req = $.VNRecognizeTextRequest.alloc.init;
  req.recognitionLevel = 0; // VNRequestTextRecognitionLevelAccurate
  req.usesLanguageCorrection = true;
  if (lang) {
    req.recognitionLanguages = $([lang]);
  } else if (req.respondsToSelector('setAutomaticallyDetectsLanguage:')) {
    req.automaticallyDetectsLanguage = true; // macOS 13+
  }
  const handler = $.VNImageRequestHandler.alloc.initWithURLOptions($.NSURL.fileURLWithPath(path), $({}));
  const err = Ref();
  if (!handler.performRequestsError($([req]), err)) {
    const e = err[0];
    throw new Error(e && !e.isNil() ? ObjC.unwrap(e.localizedDescription) : 'Vision could not read the image.');
  }
  const lines = [];
  const results = req.results;
  const n = results.isNil() ? 0 : results.count;
  for (let i = 0; i < n; i++) {
    const obs = results.objectAtIndex(i);
    const top = obs.topCandidates(1);
    if (top.count === 0) continue;
    // boundingBox is normalised with the origin at the bottom left.
    const b = obs.boundingBox;
    lines.push({
      text: ObjC.unwrap(top.objectAtIndex(0).string),
      x: Math.round(b.origin.x * width),
      y: Math.round((1 - b.origin.y - b.size.height) * height),
      w: Math.round(b.size.width * width),
      h: Math.round(b.size.height * height),
    });
  }
  return JSON.stringify({ lang: lang || 'auto', lines });
}
