(function () {
  if (window.__aegis) {
    return;
  }

  let nextId = 1;
  const queue = [];
  const idByNode = new WeakMap();
  const nodeById = new Map();
  const attrAllowList = new Set([
    "id",
    "class",
    "name",
    "type",
    "value",
    "placeholder",
    "title",
    "href",
    "src",
    "alt",
    "for",
    "autocomplete",
    "disabled",
    "readonly",
    "checked",
    "role",
    "aria-label",
    "aria-current",
    "aria-selected",
    "aria-expanded",
    "aria-pressed",
    "aria-describedby",
    "aria-disabled",
    "aria-valuemin",
    "aria-valuemax",
    "aria-valuenow",
    "data-testid",
    "data-test-id",
    "min",
    "max",
    "step"
  ]);
  const ignoredTags = new Set(["script", "style", "meta", "link", "noscript", "template"]);
  const semanticTextTags = new Set([
    "a",
    "button",
    "label",
    "option",
    "summary",
    "textarea",
    "title",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "p",
    "li",
    "dt",
    "dd",
    "th",
    "td",
    "legend"
  ]);
  const semanticTextRoles = new Set(["button", "link", "textbox", "searchbox", "option", "tab", "combobox"]);
  const mediaStateByNode = new WeakMap();
  const mediaInstrumentedNodes = new WeakSet();
  const pageBootstrapState = {
    document_loaded_at_ms: null,
    dom_mutation_count: 0,
    dom_mutation_after_load_count: 0,
    body_mutation_after_load_count: 0,
    root_mutation_after_load_count: 0,
    script_error_count: 0,
    last_script_error: null,
    unhandled_rejection_count: 0,
    last_unhandled_rejection: null
  };
  let mediaPrototypePatched = false;

  function normalizeText(value) {
    return String(value || "").replace(/\s+/g, " ").trim();
  }

  function truncateText(value, maxLength = 200) {
    return value.length > maxLength ? value.slice(0, maxLength) : value;
  }

  function currentPageState() {
    return {
      url: window.location ? window.location.href : null,
      title: document.title || null,
      ready_state: document.readyState || null,
      scroll_x: Number.isFinite(window.scrollX) ? window.scrollX : 0,
      scroll_y: Number.isFinite(window.scrollY) ? window.scrollY : 0,
      viewport: {
        width: Number.isFinite(window.innerWidth) ? window.innerWidth : null,
        height: Number.isFinite(window.innerHeight) ? window.innerHeight : null
      },
      bootstrap: pageBootstrapDiagnostics(),
      media: mediaDiagnostics()
    };
  }

  function documentLoaded() {
    return document.readyState === "interactive" || document.readyState === "complete";
  }

  function markDocumentLoaded() {
    if (documentLoaded() && !pageBootstrapState.document_loaded_at_ms) {
      pageBootstrapState.document_loaded_at_ms = Date.now();
    }
  }

  function rootCandidate() {
    return document.getElementById("root")
      || document.getElementById("app")
      || document.querySelector("[data-reactroot]")
      || null;
  }

  function rootSelectorName(root) {
    if (!root) {
      return null;
    }
    if (root.id) {
      return `#${root.id}`;
    }
    if (root.hasAttribute && root.hasAttribute("data-reactroot")) {
      return "[data-reactroot]";
    }
    return root.tagName ? root.tagName.toLowerCase() : "root";
  }

  function pageBootstrapDiagnostics() {
    markDocumentLoaded();
    const root = rootCandidate();
    const body = document.body || null;
    const pageUrl = window.location ? window.location.href : "";
    const syntheticShellActive = pageUrl === "https://bootstrap.aegis/"
      || pageUrl === "aegis://bootstrap/"
      || pageUrl === "data:text/html,";
    const bodyText = normalizeText(body && (body.innerText || body.textContent || ""));
    const rootText = normalizeText(root && (root.innerText || root.textContent || ""));
    const rootHtml = root && typeof root.innerHTML === "string" ? root.innerHTML.trim() : "";
    const moduleScripts = Array.from(document.querySelectorAll("script[type='module']"));
    const bodyDescendantCount = body && typeof body.querySelectorAll === "function"
      ? body.querySelectorAll("*").length
      : 0;
    const rootChildElementCount = root && Number.isFinite(root.childElementCount)
      ? root.childElementCount
      : 0;
    const document_loaded = documentLoaded();
    const app_dom_mutated_after_load = pageBootstrapState.dom_mutation_after_load_count > 0;
    const inspectable_dom_ready = !!(
      !syntheticShellActive &&
      document_loaded && (
        bodyText.length > 0
        || bodyDescendantCount > 3
        || rootText.length > 0
        || rootHtml.length > 0
        || rootChildElementCount > 0
        || app_dom_mutated_after_load
      )
    );
    const module_scripts_present = moduleScripts.length > 0;
    const module_bootstrap_observed = !!(
      module_scripts_present && (
        app_dom_mutated_after_load
        || rootChildElementCount > 0
        || rootText.length > 0
        || bodyDescendantCount > 3
      )
    );
    return {
      document_loaded,
      document_loaded_at_ms: pageBootstrapState.document_loaded_at_ms,
      module_scripts_present,
      module_script_count: moduleScripts.length,
      module_script_sources: moduleScripts
        .map((script) => script.getAttribute("src") || "<inline-module>")
        .slice(0, 8),
      synthetic_shell_active: syntheticShellActive,
      root_selector: rootSelectorName(root),
      root_present: !!root,
      root_child_element_count: rootChildElementCount,
      root_text_length: rootText.length,
      root_html_length: rootHtml.length,
      body_text_length: bodyText.length,
      body_descendant_count: bodyDescendantCount,
      dom_mutation_count: pageBootstrapState.dom_mutation_count,
      app_dom_mutated_after_load,
      body_mutation_after_load_count: pageBootstrapState.body_mutation_after_load_count,
      root_mutation_after_load_count: pageBootstrapState.root_mutation_after_load_count,
      module_bootstrap_observed,
      inspectable_dom_ready,
      script_error_count: pageBootstrapState.script_error_count,
      last_script_error: pageBootstrapState.last_script_error,
      unhandled_rejection_count: pageBootstrapState.unhandled_rejection_count,
      last_unhandled_rejection: pageBootstrapState.last_unhandled_rejection
    };
  }

  function finiteNumberOrNull(value) {
    return Number.isFinite(value) ? value : null;
  }

  function rectSnapshot(rect) {
    if (!rect) {
      return null;
    }
    return {
      x: finiteNumberOrNull(rect.x),
      y: finiteNumberOrNull(rect.y),
      left: finiteNumberOrNull(rect.left),
      top: finiteNumberOrNull(rect.top),
      right: finiteNumberOrNull(rect.right),
      bottom: finiteNumberOrNull(rect.bottom),
      width: finiteNumberOrNull(rect.width),
      height: finiteNumberOrNull(rect.height)
    };
  }

  function elementGeometry(node) {
    if (!node || node.nodeType !== Node.ELEMENT_NODE || typeof node.getBoundingClientRect !== "function") {
      return null;
    }
    return {
      rect: rectSnapshot(node.getBoundingClientRect()),
      client_width: finiteNumberOrNull(node.clientWidth),
      client_height: finiteNumberOrNull(node.clientHeight),
      scroll_width: finiteNumberOrNull(node.scrollWidth),
      scroll_height: finiteNumberOrNull(node.scrollHeight),
      scroll_left: finiteNumberOrNull(node.scrollLeft),
      scroll_top: finiteNumberOrNull(node.scrollTop)
    };
  }

  function trimArray(values, limit) {
    if (values.length > limit) {
      values.splice(0, values.length - limit);
    }
  }

  function codecProbeNode(tagName) {
    try {
      return document.createElement(tagName);
    } catch (_error) {
      return null;
    }
  }

  function canPlayTypeSafe(node, mimeType) {
    if (!node || typeof node.canPlayType !== "function") {
      return null;
    }
    try {
      const result = node.canPlayType(mimeType);
      return typeof result === "string" ? result : null;
    } catch (_error) {
      return null;
    }
  }

  function mediaCodecSupport() {
    const audio = codecProbeNode("audio");
    const video = codecProbeNode("video");
    return {
      audio_mp4: canPlayTypeSafe(audio, "audio/mp4"),
      audio_mp4_aac_lc: canPlayTypeSafe(audio, 'audio/mp4; codecs="mp4a.40.2"'),
      audio_aac: canPlayTypeSafe(audio, "audio/aac"),
      audio_mpeg: canPlayTypeSafe(audio, "audio/mpeg"),
      audio_ogg_opus: canPlayTypeSafe(audio, 'audio/ogg; codecs="opus"'),
      audio_wav_pcm: canPlayTypeSafe(audio, 'audio/wav; codecs="1"'),
      video_mp4_h264_aac: canPlayTypeSafe(video, 'video/mp4; codecs="avc1.42E01E, mp4a.40.2"')
    };
  }

  function mediaResourceTiming(url) {
    if (!url || !performance || typeof performance.getEntriesByName !== "function") {
      return null;
    }
    const entries = performance.getEntriesByName(url);
    if (!Array.isArray(entries) || entries.length === 0) {
      return null;
    }
    const entry = entries[entries.length - 1];
    return {
      initiator_type: entry.initiatorType || null,
      transfer_size: Number.isFinite(entry.transferSize) ? entry.transferSize : null,
      encoded_body_size: Number.isFinite(entry.encodedBodySize) ? entry.encodedBodySize : null,
      decoded_body_size: Number.isFinite(entry.decodedBodySize) ? entry.decodedBodySize : null,
      duration_ms: Number.isFinite(entry.duration) ? entry.duration : null,
      response_end_ms: Number.isFinite(entry.responseEnd) ? entry.responseEnd : null
    };
  }

  function mediaLikelyFailure(node, state, error, codecSupport, resourceTiming) {
    if (state && state.last_play_error && /user didn't interact|user gesture|gesture/i.test(state.last_play_error)) {
      return "autoplay_policy_blocked";
    }
    if (error && error.code === 4) {
      const currentSrc = String(node.currentSrc || node.src || "").toLowerCase();
      const looksLikeAacMp4 = currentSrc.includes(".m4a") ||
        currentSrc.includes(".mp4") ||
        currentSrc.includes("audio/mp4") ||
        currentSrc.includes("mp4a") ||
        (node instanceof HTMLAudioElement && state && state.metadata_parse_attempted && !state.loaded_metadata_count);
      if (looksLikeAacMp4 && codecSupport && codecSupport.audio_mp4_aac_lc === "") {
        return "embedded_runtime_missing_aac_mp4_decoder";
      }
      if (resourceTiming && resourceTiming.transfer_size === 0 && resourceTiming.encoded_body_size === 0) {
        return "media_resource_timing_unavailable_or_empty";
      }
      return "media_format_or_decoder_rejection";
    }
    if (error && error.code === 2) {
      return "media_network_failure";
    }
    if (error && error.code === 3) {
      return "media_decode_failure";
    }
    return null;
  }

  function mediaState(node) {
    if (!node || !(node instanceof HTMLMediaElement)) {
      return null;
    }
    let state = mediaStateByNode.get(node);
    if (!state) {
      state = {
        play_attempts: 0,
        play_resolved: 0,
        play_rejected: 0,
        pause_calls: 0,
        load_calls: 0,
        loaded_metadata_count: 0,
        stalled_count: 0,
        last_event: null,
        recent_events: [],
        event_timeline: [],
        metadata_parse_attempted: false,
        last_play_error: null
      };
      mediaStateByNode.set(node, state);
    }
    return state;
  }

  function recordMediaEvent(node, eventName, details) {
    const state = mediaState(node);
    if (!state) {
      return;
    }
    state.last_event = eventName;
    const atMs = Date.now();
    const entry = `${eventName}@${atMs}`;
    state.recent_events.push(entry);
    trimArray(state.recent_events, 12);
    state.event_timeline.push({
      event: eventName,
      at_ms: atMs,
      ready_state: Number.isFinite(node.readyState) ? node.readyState : null,
      network_state: Number.isFinite(node.networkState) ? node.networkState : null,
      paused: !!node.paused,
      current_time: Number.isFinite(node.currentTime) ? node.currentTime : null,
      error: details && details.error ? details.error : null
    });
    trimArray(state.event_timeline, 32);
    if (eventName === "loadedmetadata") {
      state.loaded_metadata_count += 1;
      state.metadata_parse_attempted = true;
    }
    if (eventName === "stalled") {
      state.stalled_count += 1;
    }
    if (eventName === "loadstart" || eventName === "loadeddata" || eventName === "canplay") {
      state.metadata_parse_attempted = true;
    }
    if (eventName === "play_rejected" && details && details.error) {
      state.last_play_error = String(details.error);
    }
    queue.push({
      event: {
        type: "log",
        level: details && details.level ? details.level : "debug",
        message: `media:${eventName}`,
        data: {
          node_id: assignId(node),
          tag: node.tagName ? node.tagName.toLowerCase() : "media",
          current_src: node.currentSrc || node.src || null,
          ready_state: Number.isFinite(node.readyState) ? node.readyState : null,
          network_state: Number.isFinite(node.networkState) ? node.networkState : null,
          paused: !!node.paused,
          current_time: Number.isFinite(node.currentTime) ? node.currentTime : null,
          ...(details || {})
        }
      }
    });
  }

  function patchMediaPrototype() {
    if (mediaPrototypePatched || typeof HTMLMediaElement !== "function") {
      return;
    }
    mediaPrototypePatched = true;

    const originalPlay = HTMLMediaElement.prototype.play;
    const originalPause = HTMLMediaElement.prototype.pause;
    const originalLoad = HTMLMediaElement.prototype.load;

    if (typeof originalPlay === "function") {
      HTMLMediaElement.prototype.play = function (...args) {
        const state = mediaState(this);
        if (state) {
          state.play_attempts += 1;
        }
        recordMediaEvent(this, "play_call");
        try {
          const result = originalPlay.apply(this, args);
          if (result && typeof result.then === "function") {
            return result.then((value) => {
              const nextState = mediaState(this);
              if (nextState) {
                nextState.play_resolved += 1;
              }
              recordMediaEvent(this, "play_resolved");
              return value;
            }).catch((error) => {
              const nextState = mediaState(this);
              if (nextState) {
                nextState.play_rejected += 1;
              }
              recordMediaEvent(this, "play_rejected", {
                level: "warn",
                error: String(error && error.message ? error.message : error)
              });
              throw error;
            });
          }
          return result;
        } catch (error) {
          const nextState = mediaState(this);
          if (nextState) {
            nextState.play_rejected += 1;
          }
          recordMediaEvent(this, "play_rejected", {
            level: "warn",
            error: String(error && error.message ? error.message : error)
          });
          throw error;
        }
      };
    }

    if (typeof originalPause === "function") {
      HTMLMediaElement.prototype.pause = function (...args) {
        const state = mediaState(this);
        if (state) {
          state.pause_calls += 1;
        }
        recordMediaEvent(this, "pause_call");
        return originalPause.apply(this, args);
      };
    }

    if (typeof originalLoad === "function") {
      HTMLMediaElement.prototype.load = function (...args) {
        const state = mediaState(this);
        if (state) {
          state.load_calls += 1;
        }
        recordMediaEvent(this, "load_call");
        return originalLoad.apply(this, args);
      };
    }
  }

  function instrumentMediaNode(node) {
    if (!node || !(node instanceof HTMLMediaElement) || mediaInstrumentedNodes.has(node)) {
      return;
    }
    mediaInstrumentedNodes.add(node);
    const state = mediaState(node);
    if (state && node.readyState >= 1) {
      state.loaded_metadata_count = Math.max(state.loaded_metadata_count, 1);
      state.metadata_parse_attempted = true;
      if (!state.last_event) {
        state.last_event = "metadata_available";
      }
      state.recent_events.push(`metadata_available@${Date.now()}`);
      trimArray(state.recent_events, 12);
    }
    const events = [
      "loadstart",
      "loadedmetadata",
      "loadeddata",
      "canplay",
      "canplaythrough",
      "play",
      "playing",
      "pause",
      "seeking",
      "seeked",
      "stalled",
      "waiting",
      "suspend",
      "progress",
      "durationchange",
      "timeupdate",
      "ended",
      "error"
    ];
    for (const eventName of events) {
      node.addEventListener(eventName, () => {
        recordMediaEvent(node, eventName, {
          error: node.error ? `${node.error.code || "media_error"}${node.error.message ? `: ${node.error.message}` : ""}` : null
        });
      });
    }
  }

  function instrumentMediaTree(root) {
    if (!root) {
      return;
    }
    if (root instanceof HTMLMediaElement) {
      instrumentMediaNode(root);
    }
    if (root.querySelectorAll) {
      for (const node of Array.from(root.querySelectorAll("video, audio"))) {
        instrumentMediaNode(node);
      }
    }
  }

  function mediaDiagnostics() {
    patchMediaPrototype();
    instrumentMediaTree(document);
    const codecSupport = mediaCodecSupport();
    return Array.from(document.querySelectorAll("video, audio")).map((node, index) => {
      const state = mediaState(node) || {};
      const errorCode = node.error && Number.isFinite(node.error.code) ? node.error.code : null;
      const errorMessage = node.error && node.error.message ? String(node.error.message) : null;
      const error = node.error ? `${node.error.code || "media_error"}${node.error.message ? `: ${node.error.message}` : ""}` : null;
      const resourceTiming = mediaResourceTiming(node.currentSrc || node.src || null);
      const likelyFailure = mediaLikelyFailure(node, state, {
        code: errorCode,
        message: errorMessage
      }, codecSupport, resourceTiming);
      return {
        index,
        node_id: assignId(node),
        tag: node.tagName.toLowerCase(),
        current_src: node.currentSrc || node.src || null,
        source_codec_support: codecSupport,
        ready_state: Number.isFinite(node.readyState) ? node.readyState : null,
        network_state: Number.isFinite(node.networkState) ? node.networkState : null,
        duration: Number.isFinite(node.duration) ? node.duration : null,
        paused: !!node.paused,
        ended: !!node.ended,
        muted: !!node.muted,
        seeking: !!node.seeking,
        current_time: Number.isFinite(node.currentTime) ? node.currentTime : null,
        playback_rate: Number.isFinite(node.playbackRate) ? node.playbackRate : null,
        volume: Number.isFinite(node.volume) ? node.volume : null,
        loop_enabled: !!node.loop,
        autoplay: !!node.autoplay,
        controls: !!node.controls,
        play_attempts: state.play_attempts || 0,
        play_resolved: state.play_resolved || 0,
        play_rejected: state.play_rejected || 0,
        pause_calls: state.pause_calls || 0,
        load_calls: state.load_calls || 0,
        loaded_metadata_count: state.loaded_metadata_count || 0,
        metadata_parse_attempted: !!state.metadata_parse_attempted,
        stalled_count: state.stalled_count || 0,
        last_event: state.last_event || null,
        recent_events: Array.isArray(state.recent_events) ? state.recent_events.slice() : [],
        event_timeline: Array.isArray(state.event_timeline) ? state.event_timeline.slice() : [],
        resource_timing: resourceTiming,
        error,
        error_code: errorCode,
        error_message: errorMessage,
        likely_failure_cause: likelyFailure,
        last_play_error: state.last_play_error || null
      };
    });
  }

  function isIgnoredNode(node) {
    return !!node && node.nodeType === Node.ELEMENT_NODE && ignoredTags.has(node.tagName.toLowerCase());
  }

  function assignId(node) {
    if (!node || node.nodeType !== Node.ELEMENT_NODE || isIgnoredNode(node)) {
      return null;
    }
    let id = idByNode.get(node);
    if (id == null) {
      id = nextId++;
      idByNode.set(node, id);
      nodeById.set(id, node);
    }
    return id;
  }

  function isElementVisible(node) {
    if (!node || node.nodeType !== Node.ELEMENT_NODE) {
      return false;
    }
    const style = window.getComputedStyle ? window.getComputedStyle(node) : null;
    if (node.hasAttribute("hidden")) {
      return false;
    }
    if (style && (style.display === "none" || style.visibility === "hidden" || style.visibility === "collapse")) {
      return false;
    }
    if (node.getAttribute("aria-hidden") === "true") {
      return false;
    }
    const rect = typeof node.getBoundingClientRect === "function" ? node.getBoundingClientRect() : null;
    if (!rect) {
      return true;
    }
    return rect.width > 0 && rect.height > 0;
  }

  function elementValue(node) {
    if (node instanceof HTMLInputElement || node instanceof HTMLTextAreaElement || node instanceof HTMLSelectElement) {
      return node.value;
    }
    return null;
  }

  function associatedLabel(node) {
    if (!node || node.nodeType !== Node.ELEMENT_NODE) {
      return null;
    }
    if (node instanceof HTMLInputElement ||
        node instanceof HTMLTextAreaElement ||
        node instanceof HTMLSelectElement) {
      if (node.labels && node.labels.length > 0) {
        const text = normalizeText(Array.from(node.labels).map((label) => label.innerText || label.textContent || "").join(" "));
        if (text) {
          return truncateText(text);
        }
      }
    }
    if (node instanceof HTMLLabelElement) {
      const text = normalizeText(node.innerText || node.textContent || "");
      if (text) {
        return truncateText(text);
      }
    }
    return null;
  }

  function readNodeText(node, attrs) {
    if (!node || node.nodeType !== Node.ELEMENT_NODE || isIgnoredNode(node)) {
      return null;
    }

    const tag = node.tagName.toLowerCase();
    const role = (attrs.role || "").toLowerCase();
    let text = "";

    if (node instanceof HTMLInputElement) {
      text = node.value || attrs.placeholder || attrs["aria-label"] || attrs.title || "";
    } else if (node instanceof HTMLTextAreaElement || node instanceof HTMLSelectElement) {
      text = node.value || attrs.placeholder || attrs["aria-label"] || attrs.title || "";
    } else if (
      semanticTextTags.has(tag) ||
      semanticTextRoles.has(role) ||
      node.children.length === 0
    ) {
      text = node.innerText || node.textContent || "";
    }

    text = normalizeText(text);
    if (!text) {
      text = normalizeText(
        attrs["aria-label"] || attrs.title || attrs.placeholder || attrs.value || ""
      );
    }
    return text ? truncateText(text) : null;
  }

  function implicitRole(node, attrs) {
    if (!node || node.nodeType !== Node.ELEMENT_NODE) {
      return null;
    }
    const tag = node.tagName.toLowerCase();
    if (attrs.role) {
      return attrs.role;
    }
    if (tag === "a" && attrs.href) return "link";
    if (tag === "button") return "button";
    if (tag === "select") return "combobox";
    if (tag === "textarea") return "textbox";
    if (tag === "option") return "option";
    if (tag === "summary") return "button";
    if (node.isContentEditable) return "textbox";
    if (tag === "input") {
      switch ((attrs.type || "text").toLowerCase()) {
        case "search":
          return "searchbox";
        case "button":
        case "submit":
        case "reset":
        case "image":
          return "button";
        case "checkbox":
          return "checkbox";
        case "radio":
          return "radio";
        case "range":
          return "slider";
        default:
          return "textbox";
      }
    }
    return null;
  }

  function accessibleName(node, attrs, text) {
    const label = associatedLabel(node);
    const tag = node && node.nodeType === Node.ELEMENT_NODE ? node.tagName.toLowerCase() : "";
    const name = normalizeText(
      attrs["aria-label"] ||
      label ||
      ((tag === "button" || tag === "input" || tag === "textarea" || tag === "select") ? attrs.title : "") ||
      ((tag === "button" || tag === "input") ? attrs.value : "") ||
      text ||
      attrs.placeholder ||
      attrs.alt ||
      attrs.title ||
      attrs.value ||
      ""
    );
    return name ? truncateText(name) : null;
  }

  function controlType(node, attrs) {
    if (!node || node.nodeType !== Node.ELEMENT_NODE) {
      return null;
    }
    const tag = node.tagName.toLowerCase();
    if (tag === "input") {
      const type = (attrs.type || "text").toLowerCase();
      const role = (attrs.role || "").toLowerCase();
      const searchHint = `${attrs["aria-label"] || ""} ${attrs.placeholder || ""} ${attrs.name || ""}`.toLowerCase();
      if (type === "hidden") {
        return "hidden";
      }
      if (type === "search") {
        return "searchbox";
      }
      if (type === "submit" || type === "image") {
        return "submit";
      }
      if (role === "combobox" && searchHint.includes("search")) {
        return "searchbox";
      }
      if (role === "combobox") {
        return "combobox";
      }
      return type;
    }
    if (tag === "button") {
      return (attrs.type || "").toLowerCase() === "submit" ? "submit" : "button";
    }
    if (tag === "textarea" || tag === "select") {
      return tag;
    }
    if (tag === "video" || tag === "audio") {
      return "media";
    }
    if (node.isContentEditable) {
      return "textbox";
    }
    if (tag === "a" && attrs.href) {
      return "link";
    }
    return null;
  }

  function availableActions(node, attrs) {
    if (!node || node.nodeType !== Node.ELEMENT_NODE) {
      return [];
    }
    const actions = [];
    const tag = node.tagName.toLowerCase();
    const type = (attrs.type || "").toLowerCase();
    const disabled = node.hasAttribute("disabled") || attrs["aria-disabled"] === "true";
    if (disabled) {
      return actions;
    }
    if (tag === "input" && type === "hidden") {
      return actions;
    }
    if (tag === "a" && attrs.href) {
      actions.push("click", "open", "hover", "press_key");
    }
    const clickableRoles = new Set(["button", "combobox", "menuitem", "option", "tab"]);
    if (tag === "button" || clickableRoles.has(attrs.role) || (tag === "input" && ["button", "submit", "reset", "checkbox", "radio", "image"].includes(type)) || tag === "summary") {
      actions.push("click", "hover", "press_key");
    }
    if (tag === "input" && [ "submit", "image" ].includes(type)) {
      actions.push("submit");
    }
    if (tag === "button" && type === "submit") {
      actions.push("submit");
    }
    if (node instanceof HTMLInputElement || node instanceof HTMLTextAreaElement || node instanceof HTMLSelectElement || node.isContentEditable) {
      actions.push("type", "focus", "hover", "press_key");
    }
    if (tag === "video" || tag === "audio") {
      actions.push("click", "hover", "focus", "press_key");
    }
    if (tag === "label" || attrs.role || tag === "div" || tag === "span") {
      actions.push("hover");
    }
    return Array.from(new Set(actions));
  }

  function semanticInfo(node, attrs, text) {
    const role = implicitRole(node, attrs);
    const label = associatedLabel(node);
    const name = accessibleName(node, attrs, text);
    const actions = availableActions(node, attrs);
    const disabled = !!(node && node.nodeType === Node.ELEMENT_NODE &&
      (node.hasAttribute("disabled") || attrs["aria-disabled"] === "true"));
    const control = controlType(node, attrs);
    const visible = isElementVisible(node);
    if (!role && !label && !name && !control && actions.length === 0 && !disabled) {
      return null;
    }
    return {
      role,
      name,
      label,
      control_type: control,
      actionable: control !== "hidden" && actions.length > 0,
      disabled,
      visible,
      actions
    };
  }

  function serializeNode(node) {
    const id = assignId(node);
    if (id == null) {
      return null;
    }

    const attrs = {};
    for (const attr of Array.from(node.attributes || [])) {
      if (attrAllowList.has(attr.name)) {
        attrs[attr.name] = attr.value;
      }
    }
    const liveValue = elementValue(node);
    if (liveValue != null) {
      attrs.value = liveValue;
    }

    const children = [];
    for (const child of Array.from(node.children || [])) {
      const childId = assignId(child);
      if (childId != null) {
        children.push(childId);
      }
    }

    const text = readNodeText(node, attrs);
    return {
      id,
      tag: node.tagName.toLowerCase(),
      attrs,
      text,
      semantic: semanticInfo(node, attrs, text),
      children
    };
  }

  function collectSerializedSubtree(node, output) {
    if (!node || node.nodeType !== Node.ELEMENT_NODE) {
      return;
    }
    const serialized = serializeNode(node);
    if (serialized) {
      output.push({ kind: "upsert", ...serialized });
    }
    for (const child of Array.from(node.children || [])) {
      collectSerializedSubtree(child, output);
    }
  }

  function collectRemovedNodeIds(node, output) {
    if (!node || node.nodeType !== Node.ELEMENT_NODE) {
      return;
    }
    const id = idByNode.get(node);
    if (id != null) {
      output.push({ kind: "remove", id });
      nodeById.delete(id);
    }
    for (const child of Array.from(node.children || [])) {
      collectRemovedNodeIds(child, output);
    }
  }

  function snapshot() {
    if (!document.documentElement) {
      return { nodes: [] };
    }
    const nodes = [];
    const walker = document.createTreeWalker(document.documentElement, NodeFilter.SHOW_ELEMENT);
    let current = walker.currentNode;
    while (current) {
      const serialized = isIgnoredNode(current) ? null : serializeNode(current);
      if (serialized) {
        nodes.push(serialized);
      }
      current = walker.nextNode();
    }
    return { nodes };
  }

  function findById(id) {
    return nodeById.get(id) || null;
  }

  function includesNormalized(actual, expected, exactOnly) {
    const left = normalizeText(actual || "").toLowerCase();
    const right = normalizeText(expected || "").toLowerCase();
    if (!right) {
      return true;
    }
    if (!left) {
      return false;
    }
    return exactOnly ? left === right : left.includes(right);
  }

  function describeNode(node) {
    const attrs = {};
    for (const attr of Array.from(node.attributes || [])) {
      if (attrAllowList.has(attr.name)) {
        attrs[attr.name] = attr.value;
      }
    }
    const liveValue = elementValue(node);
    if (liveValue != null) {
      attrs.value = liveValue;
    }
    const text = readNodeText(node, attrs);
    const semantic = semanticInfo(node, attrs, text) || {
      role: null,
      name: null,
      label: null,
      control_type: null,
      actionable: false,
      disabled: false,
      visible: false,
      actions: []
    };
    return {
      node,
      id: assignId(node),
      tag: node.tagName ? node.tagName.toLowerCase() : "",
      attrs,
      text,
      semantic
    };
  }

  function placeholderCandidates(descriptor) {
    return [
      descriptor.attrs.placeholder || "",
      descriptor.attrs["aria-label"] || "",
      descriptor.semantic.name || "",
      descriptor.semantic.label || ""
    ].filter(Boolean);
  }

  function scorePlaceholderField(descriptor, expected, exactOnly, fields) {
    if (!expected) {
      return 0;
    }
    const candidates = placeholderCandidates(descriptor);
    if (candidates.length === 0) {
      return null;
    }
    let best = null;
    for (const candidate of candidates) {
      const score = scoreStringField("placeholder", candidate, expected, exactOnly, fields);
      if (score != null) {
        best = best == null ? score : Math.max(best, score);
      }
    }
    return best;
  }

  function isTargetableForAction(descriptor, action) {
    if (descriptor.semantic.disabled) {
      return false;
    }
    if (action === "set_files") {
      return descriptor.tag === "input" && descriptor.semantic.control_type === "file";
    }
    if (!descriptor.semantic.visible) {
      return false;
    }
    if (action === "click") {
      return descriptor.semantic.actions.includes("click") ||
        descriptor.semantic.actions.includes("open") ||
        descriptor.semantic.actions.includes("submit") ||
        descriptor.tag === "label";
    }
    if (action === "type") {
      return descriptor.semantic.actions.includes("type");
    }
    if (action === "hover") {
      return descriptor.semantic.actions.includes("hover") || descriptor.semantic.visible;
    }
    if (action === "press_key") {
      return descriptor.semantic.actions.includes("press_key") ||
        descriptor.semantic.actions.includes("focus") ||
        descriptor.semantic.actions.includes("type");
    }
    return true;
  }

  function scoreStringField(field, actual, expected, exactOnly, fields) {
    if (!expected) {
      return 0;
    }
    const left = normalizeText(actual || "");
    const right = normalizeText(expected || "");
    if (!left || !right) {
      return null;
    }
    if (left.toLowerCase() === right.toLowerCase()) {
      fields.push({ field, exact: true, score: 120 });
      return 120;
    }
    if (exactOnly || !left.toLowerCase().includes(right.toLowerCase())) {
      return null;
    }
    fields.push({ field, exact: false, score: 60 });
    return 60;
  }

  function exactStringField(actual, expected) {
    if (!expected) {
      return true;
    }
    const left = normalizeText(actual || "");
    const right = normalizeText(expected || "");
    return !!left && !!right && left.toLowerCase() === right.toLowerCase();
  }

  function exactPlaceholderField(descriptor, expected) {
    if (!expected) {
      return true;
    }
    const right = normalizeText(expected || "").toLowerCase();
    if (!right) {
      return false;
    }
    return placeholderCandidates(descriptor)
      .map((candidate) => normalizeText(candidate).toLowerCase())
      .some((candidate) => candidate === right);
  }

  function exactMatch(descriptor, matcher) {
    return exactStringField(descriptor.semantic.role, matcher.role) &&
      exactStringField(descriptor.attrs["data-testid"] || descriptor.attrs["data-test-id"], matcher.test_id) &&
      exactStringField(descriptor.semantic.name, matcher.name) &&
      exactStringField(descriptor.semantic.label, matcher.label) &&
      exactStringField(descriptor.semantic.control_type, matcher.control_type) &&
      exactStringField(descriptor.tag, matcher.tag) &&
      exactStringField(descriptor.text, matcher.text) &&
      exactPlaceholderField(descriptor, matcher.placeholder) &&
      exactStringField(descriptor.attrs.href, matcher.href_contains);
  }

  function scoreCandidate(descriptor, matcher, action) {
    if (typeof matcher.actionable === "boolean" && !!descriptor.semantic.actionable !== matcher.actionable) {
      return null;
    }
    if (typeof matcher.disabled === "boolean" && !!descriptor.semantic.disabled !== matcher.disabled) {
      return null;
    }

    if (action && !isTargetableForAction(descriptor, action)) {
      return null;
    }

    if (matcher.exact && !exactMatch(descriptor, matcher)) {
      return null;
    }

    const fields = [];
    const exactOnly = !!matcher.exact;
    let score = 0;
    const checks = [
      ["test_id", descriptor.attrs["data-testid"] || descriptor.attrs["data-test-id"], matcher.test_id],
      ["role", descriptor.semantic.role, matcher.role],
      ["name", descriptor.semantic.name, matcher.name],
      ["label", descriptor.semantic.label, matcher.label],
      ["control_type", descriptor.semantic.control_type, matcher.control_type],
      ["tag", descriptor.tag, matcher.tag],
      ["text", descriptor.text, matcher.text],
      ["href_contains", descriptor.attrs.href, matcher.href_contains]
    ];

    for (const [field, actual, expected] of checks) {
      const fieldScore = scoreStringField(field, actual, expected, exactOnly, fields);
      if (fieldScore == null) {
        return null;
      }
      score += fieldScore;
    }
    const placeholderScore = scorePlaceholderField(descriptor, matcher.placeholder, exactOnly, fields);
    if (placeholderScore == null) {
      return null;
    }
    score += placeholderScore;

    if (descriptor.semantic.visible) {
      score += 40;
    }
    if (descriptor.semantic.actionable) {
      score += 20;
    }
    if (action && isTargetableForAction(descriptor, action)) {
      score += 120;
    }
    if (descriptor.tag === "a" || descriptor.tag === "button" || descriptor.tag === "input" || descriptor.tag === "textarea" || descriptor.tag === "select") {
      score += 10;
    }

    return {
      descriptor,
      score,
      fields
    };
  }

  function resolutionDebug(candidates) {
    return candidates.slice(0, 3).map((candidate) => ({
      id: candidate.descriptor.id,
      tag: candidate.descriptor.tag,
      text: candidate.descriptor.text,
      role: candidate.descriptor.semantic.role || null,
      name: candidate.descriptor.semantic.name || null,
      control_type: candidate.descriptor.semantic.control_type || null,
      actionable: !!candidate.descriptor.semantic.actionable,
      visible: !!candidate.descriptor.semantic.visible,
      score: candidate.score,
      fields: candidate.fields
    }));
  }

  function resolveTarget(target, action) {
    if (typeof target === "number") {
      const node = findById(target);
      if (!node) {
        throw new Error(`node ${target} not found`);
      }
      const descriptor = describeNode(node);
      if (action && !isTargetableForAction(descriptor, action)) {
        throw new Error(`node ${target} is not targetable for ${action}`);
      }
      return {
        node,
        targetId: target,
        matched: false,
        debug: {
          candidate_count: 1,
          chosen: resolutionDebug([{ descriptor, score: 0, fields: [] }])[0],
          candidates: resolutionDebug([{ descriptor, score: 0, fields: [] }])
        }
      };
    }

    if (!target || typeof target !== "object") {
      throw new Error("command target must be a node id or matcher object");
    }

    const matcher = target;
    const candidates = [];
    let scopedNodes = null;
    if (matcher.selector) {
      try {
        scopedNodes = Array.from(document.querySelectorAll(String(matcher.selector)));
      } catch (error) {
        throw new Error(`invalid selector ${JSON.stringify(matcher.selector)}: ${String(error && error.message ? error.message : error)}`);
      }
      if (scopedNodes.length === 0) {
        throw new Error(`no node matched selector ${JSON.stringify(matcher.selector)}`);
      }
    }
    for (const node of scopedNodes || document.querySelectorAll("*")) {
      if (isIgnoredNode(node)) {
        continue;
      }
      const descriptor = describeNode(node);
      if (descriptor.id == null) {
        continue;
      }
      const candidate = scoreCandidate(descriptor, matcher, action);
      if (candidate) {
        candidates.push(candidate);
      }
    }

    candidates.sort((left, right) => {
      if (right.score !== left.score) {
        return right.score - left.score;
      }
      if (!!right.descriptor.semantic.actionable !== !!left.descriptor.semantic.actionable) {
        return right.descriptor.semantic.actionable ? 1 : -1;
      }
      return left.descriptor.id - right.descriptor.id;
    });

    if (candidates.length === 0) {
      throw new Error(`no node matched ${JSON.stringify(matcher)}`);
    }

    if (candidates.length > 1 && candidates[0].score === candidates[1].score) {
      throw new Error(`ambiguous node match ${JSON.stringify({
        matcher,
        candidates: resolutionDebug(candidates)
      })}`);
    }

    return {
      node: candidates[0].descriptor.node,
      targetId: candidates[0].descriptor.id,
      matched: true,
      debug: {
        candidate_count: candidates.length,
        chosen: resolutionDebug(candidates)[0],
        candidates: resolutionDebug(candidates)
      }
    };
  }

  function commandTargetValue(target) {
    if (!target || typeof target !== "object") {
      return target;
    }
    if (Object.prototype.hasOwnProperty.call(target, "match")) {
      return target.match;
    }
    if (Object.prototype.hasOwnProperty.call(target, "id")) {
      return target.id;
    }
    return target;
  }

  function resolveTargetInfo(target, action) {
    const resolved = resolveTarget(commandTargetValue(target), action || null);
    return {
      target_id: resolved.targetId,
      matched: resolved.matched,
      debug: resolved.debug
    };
  }

  function waitState(selector) {
    let selectorFound = false;
    if (selector) {
      try {
        selectorFound = !!document.querySelector(selector);
      } catch (_error) {
        selectorFound = false;
      }
    }
    const animations = typeof document.getAnimations === "function"
      ? document.getAnimations().some((animation) => animation.playState === "running")
      : false;
    return {
      scroll_x: Number.isFinite(window.scrollX) ? window.scrollX : null,
      scroll_y: Number.isFinite(window.scrollY) ? window.scrollY : null,
      selector_found: selectorFound,
      animations_running: animations
    };
  }

  function pageResearchData() {
    const title = document.title || null;
    const url = window.location ? window.location.href : null;
    const canonicalLink = document.querySelector("link[rel='canonical'][href]");
    const canonicalUrl = canonicalLink ? canonicalLink.href : null;
    const body = document.body;
    const bodyRect = body && typeof body.getBoundingClientRect === "function"
      ? body.getBoundingClientRect()
      : null;
    const bodyArea = bodyRect ? Math.max(bodyRect.width * bodyRect.height, 1) : 1;
    const visibleText = normalizeText(body ? (body.innerText || body.textContent || "") : "");
    const articleRoot = document.querySelector("article");
    const mainCandidates = [
      document.querySelector("main"),
      document.querySelector("[role='main']"),
      articleRoot,
      document.querySelector(".markdown-body"),
      document.querySelector("[data-testid='main-content']"),
      document.querySelector(".docs-content"),
      document.querySelector(".documentation"),
      document.querySelector("#content")
    ].filter((node, index, list) => !!node && list.indexOf(node) === index);
    const contentWeight = (node) => {
      if (!node || !isElementVisible(node)) {
        return -1;
      }
      const text = normalizeText(node.innerText || node.textContent || "");
      const rect = typeof node.getBoundingClientRect === "function"
        ? node.getBoundingClientRect()
        : null;
      const areaScore = rect ? Math.min((rect.width * rect.height) / bodyArea, 1) * 300 : 0;
      const headingScore = node.querySelectorAll ? node.querySelectorAll("h1, h2, h3").length * 80 : 0;
      const paragraphScore = node.querySelectorAll ? node.querySelectorAll("p, li").length * 16 : 0;
      const linkPenalty = node.querySelectorAll ? node.querySelectorAll("nav a, header a, footer a").length * 20 : 0;
      return text.length + areaScore + headingScore + paragraphScore - linkPenalty;
    };
    const mainRoot = mainCandidates
      .map((node) => ({ node, score: contentWeight(node) }))
      .sort((left, right) => right.score - left.score)[0]?.node || null;
    const selectorForNode = (node) => {
      if (!node || node.nodeType !== Node.ELEMENT_NODE) {
        return null;
      }
      if (node.id) {
        return `#${node.id}`;
      }
      const tag = node.tagName ? node.tagName.toLowerCase() : "node";
      if (node.getAttribute("role")) {
        return `${tag}[role="${node.getAttribute("role")}"]`;
      }
      if (node.classList && node.classList.length > 0) {
        return `${tag}.${Array.from(node.classList).slice(0, 2).join(".")}`;
      }
      return tag;
    };
    const inViewport = (node) => {
      if (!node || typeof node.getBoundingClientRect !== "function") {
        return false;
      }
      const rect = node.getBoundingClientRect();
      return rect.bottom >= 0 &&
        rect.right >= 0 &&
        rect.top <= window.innerHeight &&
        rect.left <= window.innerWidth;
    };
    const summarizeControl = (node, index) => {
      if (!node || !isElementVisible(node)) {
        return null;
      }
      const attrs = {};
      if (node.attributes) {
        for (const attr of Array.from(node.attributes)) {
          attrs[attr.name] = attr.value;
        }
      }
      const text = readNodeText(node, attrs) || "";
      const role = implicitRole(node, attrs);
      const controlKind = controlType(node, attrs) || role || node.tagName.toLowerCase();
      return {
        index,
        kind: controlKind,
        role,
        text,
        label: associatedLabel(node),
        placeholder: attrs.placeholder || null,
        control_type: controlType(node, attrs),
        href: node.tagName && node.tagName.toLowerCase() === "a" ? (node.href || null) : null,
        actions: availableActions(node, attrs),
        disabled: !!(node.hasAttribute("disabled") || attrs["aria-disabled"] === "true"),
        in_main_content: !!(mainRoot && mainRoot.contains(node)),
        in_viewport: inViewport(node)
      };
    };
    const headings = Array.from(document.querySelectorAll("h1, h2, h3, h4, h5, h6"))
      .map((node) => ({
        level: Number.parseInt(node.tagName.slice(1), 10) || null,
        text: normalizeText(node.innerText || node.textContent || ""),
        id: node.id || null
      }))
      .filter((heading) => heading.text);
    const linkNodes = Array.from(document.querySelectorAll("a[href]"))
      .filter((node) => isElementVisible(node));
    const links = linkNodes
      .map((node, index) => ({
        index,
        text: normalizeText(node.innerText || node.textContent || node.getAttribute("aria-label") || node.title || ""),
        href: node.href,
        title: node.title || null
      }))
      .filter((link) => link.href);
    const scoreLink = (node) => {
      let score = 0;
      if (mainRoot && mainRoot.contains(node)) {
        score += 100;
      }
      if (inViewport(node)) {
        score += 40;
      }
      score += Math.min(normalizeText(node.innerText || node.textContent || "").length, 80);
      if (node.closest("nav, header, footer")) {
        score -= 50;
      }
      return score;
    };
    const primaryLinks = linkNodes
      .map((node, index) => ({ node, index, score: scoreLink(node) }))
      .sort((left, right) => right.score - left.score)
      .slice(0, 8)
      .map(({ node, index }) => ({
        index,
        text: normalizeText(node.innerText || node.textContent || node.getAttribute("aria-label") || node.title || ""),
        href: node.href,
        title: node.title || null
      }))
      .filter((link) => link.href);
    const controlSelector = [
      "button",
      "input:not([type='hidden'])",
      "textarea",
      "select",
      "a[href]",
      "[role='button']",
      "[role='link']",
      "[role='textbox']",
      "[role='searchbox']",
      "[role='combobox']",
      "[role='checkbox']",
      "[role='radio']",
      "[contenteditable='true']"
    ].join(", ");
    const controlNodes = Array.from(document.querySelectorAll(controlSelector))
      .filter((node, index, list) => list.indexOf(node) === index);
    const controls = controlNodes
      .map((node, index) => summarizeControl(node, index))
      .filter((control) => !!control);
    const scoreControl = (control) => {
      let score = 0;
      if (control.in_main_content) {
        score += 100;
      }
      if (control.in_viewport) {
        score += 40;
      }
      if (control.kind === "searchbox" || control.kind === "submit" || control.kind === "button") {
        score += 30;
      }
      score += Math.min((control.text || control.label || "").length, 60);
      return score;
    };
    const primaryControls = controls
      .slice()
      .sort((left, right) => scoreControl(right) - scoreControl(left))
      .slice(0, 8);
    const forms = Array.from(document.querySelectorAll("form"))
      .filter((node) => isElementVisible(node))
      .map((form, index) => {
        const formControls = Array.from(form.querySelectorAll(controlSelector))
          .map((node, controlIndex) => summarizeControl(node, controlIndex))
          .filter((control) => !!control && control.kind !== "link");
        const submitLabels = formControls
          .filter((control) => control.control_type === "submit" || control.kind === "submit" || control.kind === "button")
          .map((control) => control.text || control.label || "")
          .filter((value) => !!value);
        return {
          index,
          id: form.id || null,
          name: form.getAttribute("name") || null,
          action: form.getAttribute("action") || null,
          method: form.getAttribute("method") || null,
          controls: formControls,
          submit_labels: submitLabels
        };
      });
    const overlayNodes = Array.from(document.querySelectorAll("body *"))
      .filter((node) => node.nodeType === Node.ELEMENT_NODE && isElementVisible(node))
      .filter((node) => {
        const style = window.getComputedStyle ? window.getComputedStyle(node) : null;
        if (!style) {
          return false;
        }
        const role = (node.getAttribute("role") || "").toLowerCase();
        const modalHint = node.getAttribute("aria-modal") === "true";
        const text = normalizeText(node.innerText || node.textContent || "");
        const overlayLike = style.position === "fixed" || style.position === "sticky" || role === "dialog" || role === "alertdialog" || modalHint;
        return overlayLike && text.length >= 20;
      });
    const overlays = overlayNodes.slice(0, 8).map((node, index) => {
      const text = normalizeText(node.innerText || node.textContent || "");
      const role = (node.getAttribute("role") || "").toLowerCase();
      const cookieLike = /cookie|consent|privacy/i.test(text);
      const authLike = /sign in|log in|continue with|authenticate/i.test(text);
      return {
        index,
        kind: role || (cookieLike ? "cookie_banner" : "overlay"),
        text,
        blocking: cookieLike || authLike || role === "dialog" || node.getAttribute("aria-modal") === "true",
        dismiss_labels: Array.from(node.querySelectorAll("button, [role='button'], a[href]"))
          .map((child) => normalizeText(child.innerText || child.textContent || child.getAttribute("aria-label") || ""))
          .filter((value) => !!value)
          .slice(0, 6)
      };
    });
    const mainText = normalizeText(mainRoot ? (mainRoot.innerText || mainRoot.textContent || "") : "");
    const articleText = normalizeText(articleRoot ? (articleRoot.innerText || articleRoot.textContent || "") : "");
    const controlsText = normalizeText(
      controls
        .map((control) => [control.kind, control.label || "", control.text || "", control.placeholder || ""]
          .filter((value) => !!value)
          .join(": "))
        .join("\n")
    );
    const overlayText = normalizeText(overlays.map((overlay) => overlay.text).join("\n"));

    const combinedText = normalizeText(`${title || ""} ${visibleText}`);
    const currentUrlWithoutHash = (() => {
      if (!url) {
        return null;
      }
      try {
        const parsed = new URL(url);
        parsed.hash = "";
        return parsed.href;
      } catch (_error) {
        return null;
      }
    })();
    const samePageAnchorLinkCount = currentUrlWithoutHash
      ? links.filter((link) => link.href && link.href.startsWith(`${currentUrlWithoutHash}#`)).length
      : 0;
    const codeBlockCount = document.querySelectorAll("pre, code").length;
    const tableCount = document.querySelectorAll("table").length;
    const documentationLikely = articleText.length >= 300
      || mainText.length >= 300
      || (
        visibleText.length >= 1000
        && headings.length >= 3
        && (samePageAnchorLinkCount >= 3 || codeBlockCount > 0 || tableCount > 0)
      );
    const headingText = normalizeText(headings.slice(0, 3).map((heading) => heading.text).join(" "));
    const shortVisibleText = visibleText.length <= 600 ? visibleText : "";
    const errorSurfaceText = normalizeText(
      Array.from(document.querySelectorAll("[role='alert'], [role='status'], .error, .not-found, #error, #not-found"))
        .filter((node) => isElementVisible(node))
        .map((node) => node.innerText || node.textContent || "")
        .join(" ")
    );
    const notFoundSurfaceText = normalizeText(`${title || ""} ${headingText} ${shortVisibleText} ${errorSurfaceText}`);
    const notFoundSignals = [];
    const pushSignal = (signal) => {
      if (!notFoundSignals.includes(signal)) {
        notFoundSignals.push(signal);
      }
    };
    if (/\b(?:404|410)\b/i.test(notFoundSurfaceText)) {
      pushSignal("status_surface_mentions_404_or_410");
    }
    if (/\bpage not found\b/i.test(notFoundSurfaceText)) {
      pushSignal("page_not_found_surface_text");
    }
    if (/\bnot found\b/i.test(notFoundSurfaceText)) {
      pushSignal("not_found_surface_text");
    }
    if (/\bdoesn't exist\b/i.test(notFoundSurfaceText) || /\bdoes not exist\b/i.test(notFoundSurfaceText)) {
      pushSignal("missing_page_surface_text");
    }
    const blockerSignals = [];
    const pushBlocker = (signal) => {
      if (!blockerSignals.includes(signal)) {
        blockerSignals.push(signal);
      }
    };
    const likelyNotFound = notFoundSignals.length > 0;
    const authWallLikely = /sign in|log in|password|continue with google|two-factor|2fa/i.test(combinedText)
      || controls.some((control) => control.control_type === "password");
    const blockedByOverlay = overlays.some((overlay) => overlay.blocking);
    if (authWallLikely) {
      pushBlocker("auth_wall_likely");
    }
    if (blockedByOverlay) {
      pushBlocker("blocking_overlay_detected");
    }
    if (/cookie|consent|privacy/i.test(overlayText)) {
      pushBlocker("cookie_banner_detected");
    }
    const usefulTextAvailable = mainText.length >= 120 || visibleText.length >= 160;
    const interactiveElementsAvailable = controls.length > 0 || forms.length > 0;
    const primaryHeading = headings[0] ? headings[0].text : null;
    const pageType = likelyNotFound
      ? "not_found"
      : authWallLikely
        ? "auth"
        : /search results|results for/i.test(combinedText)
          ? "search_results"
          : documentationLikely
            ? "documentation"
            : forms.length > 0 && controls.some((control) => control.control_type === "password")
              ? "login_form"
              : controls.length >= 8 && headings.length <= 2
                ? "dashboard"
                : "content";
    let suggestedSearchQuery = null;
    if (likelyNotFound && url) {
      try {
        const host = new URL(url).hostname;
        const topic = normalizeText(primaryHeading || title || "").trim();
        suggestedSearchQuery = normalizeText(`site:${host} ${topic}`).trim() || `site:${host}`;
      } catch (_error) {
        suggestedSearchQuery = normalizeText(primaryHeading || title || "").trim() || null;
      }
    }
    const suggestedNextActions = [];
    if (suggestedSearchQuery) {
      suggestedNextActions.push(`Run a recovery search for ${suggestedSearchQuery}`);
    }
    const searchControl = primaryControls.find((control) => control.kind === "searchbox");
    if (searchControl) {
      suggestedNextActions.push(
        `Type into the search field${searchControl.label ? ` "${searchControl.label}"` : ""}`
      );
    }
    if (forms.length > 0) {
      suggestedNextActions.push("Inspect forms or fill the primary visible form");
    }
    if (primaryLinks[0] && primaryLinks[0].text) {
      suggestedNextActions.push(`Open the primary link "${primaryLinks[0].text}"`);
    }
    if (blockedByOverlay && overlays[0] && overlays[0].dismiss_labels[0]) {
      suggestedNextActions.push(`Dismiss the overlay via "${overlays[0].dismiss_labels[0]}"`);
    }

    return {
      title,
      url,
      canonical_url: canonicalUrl,
      visible_text: visibleText,
      content_scopes: {
        main_text: mainText,
        article_text: articleText,
        controls_text: controlsText,
        overlay_text: overlayText
      },
      main_content_selector: selectorForNode(mainRoot),
      headings,
      links,
      primary_links: primaryLinks,
      controls,
      primary_controls: primaryControls,
      forms,
      overlays,
      semantic_summary: {
        link_count: links.length,
        control_count: controls.length,
        form_count: forms.length,
        overlay_count: overlays.length,
        dialog_count: overlays.filter((overlay) => overlay.kind === "dialog" || overlay.kind === "alertdialog").length,
        alert_count: overlays.filter((overlay) => overlay.kind === "alert").length,
        code_block_count: codeBlockCount,
        table_count: tableCount
      },
      page_type: pageType,
      useful_text_available: usefulTextAvailable,
      interactive_elements_available: interactiveElementsAvailable,
      auth_wall_likely: authWallLikely,
      blocked_by_overlay: blockedByOverlay,
      blocker_signals: blockerSignals,
      suggested_next_actions: suggestedNextActions,
      likely_not_found: likelyNotFound,
      not_found_signals: notFoundSignals,
      suggested_search_query: suggestedSearchQuery
    };
  }

  function scrollIntoViewIfNeeded(el) {
    if (typeof el.scrollIntoView === "function") {
      el.scrollIntoView({ block: "center", inline: "center", behavior: "instant" });
    }
  }

  function focusIfPossible(el) {
    if (typeof el.focus === "function") {
      try {
        el.focus({ preventScroll: true });
      } catch (_error) {
        el.focus();
      }
    }
  }

  function resolveActionTarget(el, action) {
    if (action === "type" && el instanceof HTMLLabelElement && el.control) {
      return el.control;
    }
    if (action === "set_files" && el instanceof HTMLLabelElement && el.control) {
      return el.control;
    }
    return el;
  }

  function dispatchMouseLikeEvent(el, type, init) {
    const buttons = init && typeof init.buttons === "number"
      ? init.buttons
      : (type === "mouseup" || type === "mouseenter" || type === "mouseover") ? 0 : 1;
    const accepted = el.dispatchEvent(
      new MouseEvent(type, {
        bubbles: true,
        cancelable: true,
        composed: true,
        view: window,
        button: init && typeof init.button === "number" ? init.button : 0,
        buttons,
        clientX: init && Number.isFinite(init.clientX) ? init.clientX : 0,
        clientY: init && Number.isFinite(init.clientY) ? init.clientY : 0,
        screenX: init && Number.isFinite(init.screenX) ? init.screenX : 0,
        screenY: init && Number.isFinite(init.screenY) ? init.screenY : 0
      })
    );
    return {
      event: type,
      target: assignId(el),
      accepted
    };
  }

  function dispatchPointerLikeEvent(el, type, init) {
    if (typeof PointerEvent !== "function") {
      return {
        event: type,
        target: assignId(el),
        accepted: null,
        unsupported: true
      };
    }
    const buttons = init && typeof init.buttons === "number"
      ? init.buttons
      : (type === "pointerup" || type === "pointerenter" || type === "pointerover") ? 0 : 1;
    const accepted = el.dispatchEvent(
      new PointerEvent(type, {
        bubbles: true,
        cancelable: true,
        composed: true,
        pointerId: init && Number.isFinite(init.pointerId) ? init.pointerId : 1,
        pointerType: "mouse",
        isPrimary: true,
        button: init && typeof init.button === "number" ? init.button : 0,
        buttons,
        clientX: init && Number.isFinite(init.clientX) ? init.clientX : 0,
        clientY: init && Number.isFinite(init.clientY) ? init.clientY : 0,
        screenX: init && Number.isFinite(init.screenX) ? init.screenX : 0,
        screenY: init && Number.isFinite(init.screenY) ? init.screenY : 0
      })
    );
    return {
      event: type,
      target: assignId(el),
      accepted
    };
  }

  function centerPoint(rect) {
    return {
      x: rect.left + (rect.width / 2),
      y: rect.top + (rect.height / 2)
    };
  }

  function dragAnchorPoint(rect, handle) {
    if (!rect) {
      return { x: 0, y: 0 };
    }
    if (handle === "start" || handle === "left") {
      return { x: rect.left + Math.min(4, rect.width / 4), y: rect.top + (rect.height / 2) };
    }
    if (handle === "end" || handle === "right") {
      return { x: rect.right - Math.min(4, rect.width / 4), y: rect.top + (rect.height / 2) };
    }
    return centerPoint(rect);
  }

  function pointerInit(point, pointerId, buttons) {
    return {
      pointerId,
      buttons,
      clientX: point.x,
      clientY: point.y,
      screenX: point.x,
      screenY: point.y
    };
  }

  function baseActionResult(resolved, el, before, after) {
    const type = el instanceof HTMLInputElement || el instanceof HTMLButtonElement
      ? (el.getAttribute("type") || "").toLowerCase()
      : "";
    return {
      matched: resolved.matched,
      tag: el.tagName.toLowerCase(),
      control_type: type || el.tagName.toLowerCase(),
      actionable: true,
      target: resolved.debug.chosen,
      matcher_debug: resolved.debug,
      page_before: before,
      page_after: after,
      navigation_changed: before.url !== after.url,
      title_changed: before.title !== after.title
    };
  }

  function closestForm(el) {
    if (!el) {
      return null;
    }
    if (el.form instanceof HTMLFormElement) {
      return el.form;
    }
    return typeof el.closest === "function" ? el.closest("form") : null;
  }

  function invalidControlDiagnostics(form) {
    if (!(form instanceof HTMLFormElement)) {
      return [];
    }
    return Array.from(form.elements || [])
      .filter((control) => control && typeof control.checkValidity === "function" && !control.checkValidity())
      .slice(0, 8)
      .map((control) => ({
        tag: control.tagName ? control.tagName.toLowerCase() : null,
        type: typeof control.getAttribute === "function" ? (control.getAttribute("type") || "").toLowerCase() || null : null,
        name: typeof control.getAttribute === "function" ? control.getAttribute("name") || null : null,
        id: typeof control.getAttribute === "function" ? control.getAttribute("id") || null : null,
        label: typeof control.getAttribute === "function" ? control.getAttribute("aria-label") || null : null,
        validation_message: control.validationMessage || null
      }));
  }

  function formDiagnostics(form) {
    if (!(form instanceof HTMLFormElement)) {
      return null;
    }
    return {
      present: true,
      action: form.action || null,
      method: (form.method || "get").toLowerCase(),
      no_validate: !!form.noValidate,
      check_validity: typeof form.checkValidity === "function" ? form.checkValidity() : null,
      invalid_controls: invalidControlDiagnostics(form)
    };
  }

  function detectAuthDiagnostics() {
    const body = document.body;
    const visibleText = normalizeText(body ? (body.innerText || "") : "");
    const rawText = normalizeText(body ? (body.textContent || "") : "");
    const signals = [];
    const notes = [];
    const pushSignal = (kind, message) => {
      if (!signals.some((signal) => signal.kind === kind)) {
        signals.push({ kind, message });
      }
      if (message && !notes.includes(message)) {
        notes.push(message);
      }
    };

    if (visibleText.includes("incorrect password") || visibleText.includes("invalid password")) {
      pushSignal("credentials_rejected", "Incorrect password");
    }
    if (rawText.includes("captcha couldn't load") || rawText.includes("captcha could not load")) {
      pushSignal("captcha_blocked", "Captcha couldn't load. Refresh the page and try again.");
    } else if (rawText.includes("hcaptcha") || rawText.includes("recaptcha")) {
      pushSignal("captcha_present", "Captcha elements detected in page state.");
    }
    if (visibleText.includes("redirected you too many times") || visibleText.includes("too many redirects")) {
      pushSignal("redirect_loop", "The page reported too many redirects.");
    }
    if (visibleText.includes("upstream connect error") || visibleText.includes("connection termination")) {
      pushSignal("upstream_failure", "The page reported an upstream connection termination.");
    }

    return {
      signals,
      notes
    };
  }

  function isSubmitIntentElement(el) {
    if (!el) {
      return false;
    }
    if (el instanceof HTMLButtonElement) {
      return (el.getAttribute("type") || "submit").toLowerCase() === "submit";
    }
    if (el instanceof HTMLInputElement) {
      return ["submit", "image"].includes((el.getAttribute("type") || "").toLowerCase());
    }
    return false;
  }

  function trackFormSubmission(form, submitter, performAction) {
    const beforeForm = formDiagnostics(form);
    const submitEvent = {
      fired: false,
      default_prevented: false,
      submitter_tag: submitter && submitter.tagName ? submitter.tagName.toLowerCase() : null,
      submitter_type: submitter && typeof submitter.getAttribute === "function"
        ? (submitter.getAttribute("type") || "").toLowerCase() || null
        : null
    };
    if (!(form instanceof HTMLFormElement)) {
      performAction();
      return {
        form_present: false,
        form_before: beforeForm,
        form_after: null,
        submit_event: submitEvent
      };
    }

    const submitListener = (event) => {
      submitEvent.fired = true;
      submitEvent.default_prevented = !!event.defaultPrevented;
      if (event.submitter && event.submitter.tagName) {
        submitEvent.submitter_tag = event.submitter.tagName.toLowerCase();
        submitEvent.submitter_type = typeof event.submitter.getAttribute === "function"
          ? (event.submitter.getAttribute("type") || "").toLowerCase() || null
          : null;
      }
    };

    form.addEventListener("submit", submitListener, true);
    try {
      performAction();
    } finally {
      form.removeEventListener("submit", submitListener, true);
    }

    return {
      form_present: true,
      form_before: beforeForm,
      form_after: formDiagnostics(form),
      submit_event: submitEvent
    };
  }

  function click(target) {
    const before = currentPageState();
    const resolved = resolveTarget(target, "click");
    const el = resolveActionTarget(resolved.node, "click");
    const form = closestForm(el);
    const rect = el.getBoundingClientRect();
    const point = centerPoint(rect);
    const event_debug = [];
    scrollIntoViewIfNeeded(el);
    focusIfPossible(el);
    event_debug.push(dispatchPointerLikeEvent(el, "pointerover", pointerInit(point, 1, 0)));
    event_debug.push(dispatchMouseLikeEvent(el, "mouseover", pointerInit(point, 1, 0)));
    event_debug.push(dispatchPointerLikeEvent(el, "pointermove", pointerInit(point, 1, 0)));
    event_debug.push(dispatchMouseLikeEvent(el, "mousemove", pointerInit(point, 1, 0)));
    event_debug.push(dispatchPointerLikeEvent(el, "pointerdown", pointerInit(point, 1, 1)));
    event_debug.push(dispatchMouseLikeEvent(el, "mousedown", pointerInit(point, 1, 1)));
    event_debug.push(dispatchPointerLikeEvent(el, "pointerup", pointerInit(point, 1, 0)));
    event_debug.push(dispatchMouseLikeEvent(el, "mouseup", pointerInit(point, 1, 0)));
    const type = el instanceof HTMLInputElement || el instanceof HTMLButtonElement
      ? (el.getAttribute("type") || "").toLowerCase()
      : "";
    let default_action = "dispatch_click";
    const submit_tracking = trackFormSubmission(form, el, () => {
      if (el instanceof HTMLLabelElement && typeof el.click === "function") {
        default_action = "label_click";
        el.click();
      } else if ((el instanceof HTMLButtonElement || el instanceof HTMLInputElement) &&
          (type === "submit" || type === "image") &&
          el.form &&
          typeof el.form.requestSubmit === "function") {
        default_action = "request_submit";
        el.form.requestSubmit(el);
      } else if (typeof el.click === "function") {
        default_action = "native_click";
        el.click();
      } else {
        default_action = "dispatched_mouse_click";
        dispatchMouseLikeEvent(el, "click");
      }
    });
    const after = currentPageState();
    const auth_diagnostics = detectAuthDiagnostics();
    queue.push({
      event: {
        type: "log",
        level: "debug",
        message: "interaction:click",
        data: {
          node_id: resolved.targetId,
          tag: el.tagName.toLowerCase(),
          before_url: before.url,
          after_url: after.url,
          navigation_changed: before.url !== after.url,
          title_changed: before.title !== after.title,
          submit_tracking,
          auth_diagnostics
        }
      }
    });
    return {
      clicked: resolved.targetId,
      geometry_after: elementGeometry(el),
      event_debug,
      submit_intent: isSubmitIntentElement(el),
      default_action,
      submit_tracking,
      auth_diagnostics,
      ...baseActionResult(resolved, el, before, after)
    };
  }

  function setNativeElementValue(el, value) {
    const prototype =
      el instanceof HTMLInputElement
        ? HTMLInputElement.prototype
        : el instanceof HTMLTextAreaElement
          ? HTMLTextAreaElement.prototype
          : el instanceof HTMLSelectElement
            ? HTMLSelectElement.prototype
            : null;
    const descriptor = prototype && Object.getOwnPropertyDescriptor(prototype, "value");
    if (descriptor && typeof descriptor.set === "function") {
      descriptor.set.call(el, value);
      return;
    }
    el.value = value;
  }

  function emitValueMutationEvents(el, value, inputType) {
    if (typeof InputEvent === "function") {
      el.dispatchEvent(
        new InputEvent("input", {
          bubbles: true,
          composed: true,
          inputType: inputType || "insertText",
          data: value == null ? null : String(value)
        })
      );
    } else {
      el.dispatchEvent(new Event("input", { bubbles: true }));
    }
    el.dispatchEvent(new Event("change", { bubbles: true }));
  }

  function updateTrackedElementValue(el, nextValue) {
    const previousValue = "value" in el ? el.value : "";
    setNativeElementValue(el, nextValue);
    if (el._valueTracker && typeof el._valueTracker.setValue === "function") {
      el._valueTracker.setValue(previousValue);
    }
    return previousValue;
  }

  function clamp(value, minimum, maximum) {
    return Math.min(maximum, Math.max(minimum, value));
  }

  function numericAttr(el, name, fallback) {
    const raw = el.getAttribute(name);
    const parsed = raw == null || raw == "" ? NaN : Number(raw);
    return Number.isFinite(parsed) ? parsed : fallback;
  }

  function applyRangeDragValue(el, endPoint) {
    if (!(el instanceof HTMLInputElement) || (el.getAttribute("type") || "").toLowerCase() !== "range") {
      return null;
    }

    const rect = el.getBoundingClientRect();
    if (!rect || rect.width <= 0) {
      return null;
    }

    const min = numericAttr(el, "min", 0);
    const max = numericAttr(el, "max", 100);
    const stepAttr = (el.getAttribute("step") || "").toLowerCase();
    const step = stepAttr && stepAttr !== "any" ? numericAttr(el, "step", 1) : null;
    const ratio = clamp((endPoint.x - rect.left) / rect.width, 0, 1);
    let next = min + ((max - min) * ratio);
    if (step && step > 0) {
      next = min + (Math.round((next - min) / step) * step);
    }
    next = clamp(next, min, max);

    const normalized = step && step > 0
      ? String(Number(next.toFixed(6)))
      : String(next);
    updateTrackedElementValue(el, normalized);
    emitValueMutationEvents(el, normalized, "insertReplacementText");
    return {
      applied: true,
      mode: "native_range",
      value: normalized,
      min,
      max,
      step
    };
  }

  function dispatchDocumentPointerMove(point, pointerId, event_debug) {
    const init = pointerInit(point, pointerId, 1);
    event_debug.push(dispatchPointerLikeEvent(document, "pointermove", init));
    event_debug.push(dispatchMouseLikeEvent(document, "mousemove", init));
    if (window && typeof window.dispatchEvent === "function") {
      event_debug.push({
        event: "window:pointermove",
        target: null,
        accepted: window.dispatchEvent(new PointerEvent("pointermove", {
          bubbles: true,
          cancelable: true,
          composed: true,
          pointerId,
          pointerType: "mouse",
          isPrimary: true,
          button: 0,
          buttons: 1,
          clientX: point.x,
          clientY: point.y,
          screenX: point.x,
          screenY: point.y
        }))
      });
      event_debug.push({
        event: "window:mousemove",
        target: null,
        accepted: window.dispatchEvent(new MouseEvent("mousemove", {
          bubbles: true,
          cancelable: true,
          composed: true,
          view: window,
          button: 0,
          buttons: 1,
          clientX: point.x,
          clientY: point.y,
          screenX: point.x,
          screenY: point.y
        }))
      });
    }
  }

  function setValue(target, value) {
    const before = currentPageState();
    const resolved = resolveTarget(target, "type");
    const el = resolveActionTarget(resolved.node, "type");
    scrollIntoViewIfNeeded(el);
    focusIfPossible(el);
    const previousValue = "value" in el ? el.value : "";

    if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) {
      setNativeElementValue(el, value);
      if (el._valueTracker && typeof el._valueTracker.setValue === "function") {
        el._valueTracker.setValue(previousValue);
      }
    } else if (el.isContentEditable) {
      el.textContent = value;
    } else if ("value" in el) {
      el.value = value;
    } else {
      throw new Error(`node ${resolved.targetId} is not typeable`);
    }

    emitValueMutationEvents(el, value, "insertText");
    const after = currentPageState();
    return {
      id: resolved.targetId,
      value,
      ...baseActionResult(resolved, el, before, after)
    };
  }

  function decodeBase64(base64) {
    const binary = atob(String(base64 || ""));
    const bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index += 1) {
      bytes[index] = binary.charCodeAt(index);
    }
    return bytes;
  }

  function setFiles(target, files) {
    const before = currentPageState();
    const resolved = resolveTarget(target, "set_files");
    const el = resolveActionTarget(resolved.node, "set_files");
    if (!(el instanceof HTMLInputElement) || (el.getAttribute("type") || "").toLowerCase() !== "file") {
      throw new Error(`node ${resolved.targetId} is not a file input`);
    }
    if (typeof DataTransfer !== "function") {
      throw new Error("DataTransfer is not available in this runtime");
    }
    if (isElementVisible(el)) {
      scrollIntoViewIfNeeded(el);
      focusIfPossible(el);
    }
    const dataTransfer = new DataTransfer();
    for (const file of Array.isArray(files) ? files : []) {
      const bytes = decodeBase64(file.base64);
      dataTransfer.items.add(new File([bytes], file.name || "upload.bin", {
        type: file.mime_type || "application/octet-stream",
        lastModified: Number.isFinite(file.last_modified_ms) ? file.last_modified_ms : Date.now()
      }));
    }
    el.files = dataTransfer.files;
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
    const after = currentPageState();
    return {
      id: resolved.targetId,
      file_count: el.files ? el.files.length : 0,
      files: Array.from(el.files || []).map((file) => ({
        name: file.name,
        size: file.size,
        type: file.type || null,
        last_modified_ms: Number.isFinite(file.lastModified) ? file.lastModified : null
      })),
      ...baseActionResult(resolved, el, before, after)
    };
  }

  function hover(target) {
    const before = currentPageState();
    const resolved = resolveTarget(target, "hover");
    const el = resolveActionTarget(resolved.node, "hover");
    const rect = el.getBoundingClientRect();
    const point = centerPoint(rect);
    const event_debug = [];
    scrollIntoViewIfNeeded(el);
    event_debug.push(dispatchPointerLikeEvent(el, "pointerover", pointerInit(point, 1, 0)));
    event_debug.push(dispatchMouseLikeEvent(el, "mouseover", pointerInit(point, 1, 0)));
    event_debug.push(dispatchPointerLikeEvent(el, "pointerenter", pointerInit(point, 1, 0)));
    event_debug.push(dispatchMouseLikeEvent(el, "mouseenter", pointerInit(point, 1, 0)));
    event_debug.push(dispatchPointerLikeEvent(el, "pointermove", pointerInit(point, 1, 0)));
    event_debug.push(dispatchMouseLikeEvent(el, "mousemove", pointerInit(point, 1, 0)));
    const after = currentPageState();
    return {
      hovered: resolved.targetId,
      geometry_after: elementGeometry(el),
      event_debug,
      ...baseActionResult(resolved, el, before, after)
    };
  }

  function dispatchKeyEvent(el, type, options) {
    const event = new KeyboardEvent(type, {
      bubbles: true,
      cancelable: true,
      composed: true,
      key: options.key,
      code: options.code || options.key,
      altKey: !!options.altKey,
      ctrlKey: !!options.ctrlKey,
      metaKey: !!options.metaKey,
      shiftKey: !!options.shiftKey
    });
    return {
      accepted: el.dispatchEvent(event),
      target: assignId(el),
      event: type
    };
  }

  function pressKeyDefaultAction(el, key, eventOptions) {
    const normalizedKey = key === "Spacebar" || key === "Space" ? " " : key;
    let triggeredClick = false;
    let triggeredSubmit = false;
    let mediaToggled = false;
    const isSummaryElement = typeof HTMLSummaryElement === "function" && el instanceof HTMLSummaryElement;

    if (normalizedKey === "Enter") {
      if (el.form && typeof el.form.requestSubmit === "function" && !(el instanceof HTMLTextAreaElement)) {
        el.form.requestSubmit();
        triggeredSubmit = true;
      } else if ((el instanceof HTMLButtonElement || el instanceof HTMLAnchorElement || el instanceof HTMLLabelElement) && typeof el.click === "function") {
        el.click();
        triggeredClick = true;
      }
    }

    if (normalizedKey === " ") {
      if ((el instanceof HTMLButtonElement || el instanceof HTMLInputElement || el instanceof HTMLLabelElement || isSummaryElement) && typeof el.click === "function") {
        el.click();
        triggeredClick = true;
      } else if (el instanceof HTMLMediaElement) {
        if (el.paused) {
          try {
            el.play();
          } catch (_error) {
          }
        } else {
          el.pause();
        }
        mediaToggled = true;
      }
    }

    return {
      triggered_click: triggeredClick,
      triggered_submit: triggeredSubmit,
      media_toggled: mediaToggled,
      modifiers: {
        alt: !!eventOptions.altKey,
        ctrl: !!eventOptions.ctrlKey,
        meta: !!eventOptions.metaKey,
        shift: !!eventOptions.shiftKey
      }
    };
  }

  function pressKey(target, key, options) {
    const before = currentPageState();
    let resolved = null;
    let el = document.activeElement || document.body;
    if (target != null) {
      resolved = resolveTarget(target, "press_key");
      el = resolveActionTarget(resolved.node, "press_key");
      scrollIntoViewIfNeeded(el);
      focusIfPossible(el);
    } else {
      focusIfPossible(el);
    }

    const eventOptions = {
      key,
      code: options && options.code ? options.code : key,
      altKey: !!(options && options.altKey),
      ctrlKey: !!(options && options.ctrlKey),
      metaKey: !!(options && options.metaKey),
      shiftKey: !!(options && options.shiftKey)
    };
    const form = closestForm(el);

    const event_debug = [];
    const keydown = dispatchKeyEvent(el, "keydown", eventOptions);
    event_debug.push(keydown);
    const keypress = dispatchKeyEvent(el, "keypress", eventOptions);
    event_debug.push(keypress);
    let default_action = {
      triggered_click: false,
      triggered_submit: false,
      media_toggled: false,
      modifiers: {
        alt: !!eventOptions.altKey,
        ctrl: !!eventOptions.ctrlKey,
        meta: !!eventOptions.metaKey,
        shift: !!eventOptions.shiftKey
      }
    };
    let submit_tracking = {
      form_present: false,
      form_before: formDiagnostics(form),
      form_after: null,
      submit_event: {
        fired: false,
        default_prevented: false,
        submitter_tag: null,
        submitter_type: null
      }
    };
    if (keydown.accepted && keypress.accepted) {
      submit_tracking = trackFormSubmission(form, el, () => {
        default_action = pressKeyDefaultAction(el, key, eventOptions);
      });
    }
    event_debug.push(dispatchKeyEvent(el, "keyup", eventOptions));
    const after = currentPageState();
    const auth_diagnostics = detectAuthDiagnostics();
    queue.push({
      event: {
        type: "log",
        level: "debug",
        message: "interaction:press_key",
        data: {
          key,
          code: eventOptions.code,
          node_id: resolved ? resolved.targetId : assignId(el),
          navigation_changed: before.url !== after.url,
          title_changed: before.title !== after.title,
          default_action,
          submit_tracking,
          auth_diagnostics
        }
      }
    });
    return {
      key,
      code: eventOptions.code,
      triggered_submit: default_action.triggered_submit,
      triggered_click: default_action.triggered_click,
      media_toggled: default_action.media_toggled,
      target: resolved ? resolved.debug.chosen : null,
      matcher_debug: resolved ? resolved.debug : null,
      event_debug,
      default_action,
      submit_intent: key === "Enter" && !!form,
      submit_tracking,
      auth_diagnostics,
      page_before: before,
      page_after: after,
      navigation_changed: before.url !== after.url,
      title_changed: before.title !== after.title
    };
  }

  function scrollToPosition(x, y) {
    const nextX = Number.isFinite(x) ? x : window.scrollX;
    const nextY = Number.isFinite(y) ? y : window.scrollY;
    window.scrollTo({ left: nextX, top: nextY, behavior: "instant" });
    return { x: window.scrollX, y: window.scrollY };
  }

  function geometry(target) {
    const resolved = resolveTarget(target, null);
    const el = resolved.node;
    return {
      id: resolved.targetId,
      tag: el.tagName.toLowerCase(),
      target: resolved.debug.chosen,
      matcher_debug: resolved.debug,
      geometry: elementGeometry(el),
      page: currentPageState()
    };
  }

  function drag(target, options) {
    const before = currentPageState();
    const resolved = resolveTarget(target, "hover");
    const el = resolveActionTarget(resolved.node, "hover");
    scrollIntoViewIfNeeded(el);
    focusIfPossible(el);

    const startRect = el.getBoundingClientRect();
    const startPoint = dragAnchorPoint(startRect, options && options.handle ? String(options.handle).toLowerCase() : null);
    const endPoint = {
      x: Number.isFinite(options && options.toX) ? options.toX : startPoint.x + (Number.isFinite(options && options.deltaX) ? options.deltaX : 0),
      y: Number.isFinite(options && options.toY) ? options.toY : startPoint.y + (Number.isFinite(options && options.deltaY) ? options.deltaY : 0)
    };
    if (!Number.isFinite(endPoint.x) || !Number.isFinite(endPoint.y)) {
      throw new Error("drag requires toX/toY or deltaX/deltaY");
    }

    const pointerId = 1;
    const steps = Math.max(1, Math.min(60, Math.round(Number.isFinite(options && options.steps) ? options.steps : 12)));
    const event_debug = [];
    const beforeGeometry = elementGeometry(el);
    const beforeValue = "value" in el ? el.value : null;

    event_debug.push(dispatchPointerLikeEvent(el, "pointerover", pointerInit(startPoint, pointerId, 0)));
    event_debug.push(dispatchMouseLikeEvent(el, "mouseover", pointerInit(startPoint, pointerId, 0)));
    event_debug.push(dispatchPointerLikeEvent(el, "pointermove", pointerInit(startPoint, pointerId, 0)));
    event_debug.push(dispatchMouseLikeEvent(el, "mousemove", pointerInit(startPoint, pointerId, 0)));
    event_debug.push(dispatchPointerLikeEvent(el, "pointerdown", pointerInit(startPoint, pointerId, 1)));
    event_debug.push(dispatchMouseLikeEvent(el, "mousedown", pointerInit(startPoint, pointerId, 1)));
    if (typeof el.setPointerCapture === "function") {
      try {
        el.setPointerCapture(pointerId);
        event_debug.push({ event: "setPointerCapture", target: assignId(el), accepted: true });
      } catch (_error) {
        event_debug.push({ event: "setPointerCapture", target: assignId(el), accepted: false });
      }
    }

    for (let index = 1; index <= steps; index += 1) {
      const progress = index / steps;
      const point = {
        x: startPoint.x + ((endPoint.x - startPoint.x) * progress),
        y: startPoint.y + ((endPoint.y - startPoint.y) * progress)
      };
      const moveTarget = document.elementFromPoint(point.x, point.y) || document.body || el;
      dispatchDocumentPointerMove(point, pointerId, event_debug);
      if (moveTarget !== document) {
        event_debug.push(dispatchPointerLikeEvent(moveTarget, "pointermove", pointerInit(point, pointerId, 1)));
        event_debug.push(dispatchMouseLikeEvent(moveTarget, "mousemove", pointerInit(point, pointerId, 1)));
      }
      if (moveTarget !== el) {
        event_debug.push(dispatchPointerLikeEvent(el, "pointermove", pointerInit(point, pointerId, 1)));
        event_debug.push(dispatchMouseLikeEvent(el, "mousemove", pointerInit(point, pointerId, 1)));
      }
    }

    const range_update = applyRangeDragValue(el, endPoint);

    const releaseTarget = document.elementFromPoint(endPoint.x, endPoint.y) || document.body || el;
    event_debug.push(dispatchPointerLikeEvent(releaseTarget, "pointerup", pointerInit(endPoint, pointerId, 0)));
    event_debug.push(dispatchMouseLikeEvent(releaseTarget, "mouseup", pointerInit(endPoint, pointerId, 0)));
    if (releaseTarget !== el) {
      event_debug.push(dispatchPointerLikeEvent(el, "pointerup", pointerInit(endPoint, pointerId, 0)));
      event_debug.push(dispatchMouseLikeEvent(el, "mouseup", pointerInit(endPoint, pointerId, 0)));
    }
    if (typeof el.releasePointerCapture === "function") {
      try {
        if (el.hasPointerCapture && el.hasPointerCapture(pointerId)) {
          el.releasePointerCapture(pointerId);
        }
      } catch (_error) {
      }
    }

    const after = currentPageState();
    queue.push({
      event: {
        type: "log",
        level: "debug",
        message: "interaction:drag",
        data: {
          node_id: resolved.targetId,
          start_point: startPoint,
          end_point: endPoint,
          before_scroll: { x: before.scroll_x, y: before.scroll_y },
          after_scroll: { x: after.scroll_x, y: after.scroll_y }
        }
      }
    });
    const afterGeometry = elementGeometry(el);
    return {
      dragged: resolved.targetId,
      requested: {
        handle: options && options.handle ? options.handle : null,
        delta_x: Number.isFinite(options && options.deltaX) ? options.deltaX : null,
        delta_y: Number.isFinite(options && options.deltaY) ? options.deltaY : null,
        to_x: Number.isFinite(options && options.toX) ? options.toX : null,
        to_y: Number.isFinite(options && options.toY) ? options.toY : null,
        steps
      },
      start_point: startPoint,
      end_point: endPoint,
      pointer_capture: typeof el.hasPointerCapture === "function" ? el.hasPointerCapture(pointerId) : null,
      range_update,
      value_before: beforeValue,
      value_after: "value" in el ? el.value : null,
      geometry_before: beforeGeometry,
      geometry_after: afterGeometry,
      scroll_before: { x: before.scroll_x, y: before.scroll_y },
      scroll_after: { x: after.scroll_x, y: after.scroll_y },
      event_debug,
      ...baseActionResult(resolved, el, before, after)
    };
  }

  function exec(commands) {
    return commands.map((command, index) => {
      const eventStart = queue.length;
      try {
        let result;
        switch (command.type) {
          case "click":
            result = { ok: true, value: click(command.match || command.id) };
            break;
          case "hover":
            result = { ok: true, value: hover(command.match || command.id) };
            break;
          case "set_value":
            result = { ok: true, value: setValue(command.match || command.id, command.value) };
            break;
          case "set_files":
            result = {
              ok: true,
              value: setFiles(command.match || command.id, command.files || [])
            };
            break;
          case "press_key":
            result = {
              ok: true,
              value: pressKey(
                command.target ? (command.target.match || command.target.id) : null,
                command.key,
                {
                  code: command.code,
                  altKey: command.alt_key,
                  ctrlKey: command.ctrl_key,
                  metaKey: command.meta_key,
                  shiftKey: command.shift_key
                }
              )
            };
            break;
          case "scroll":
            result = { ok: true, value: scrollToPosition(command.x, command.y) };
            break;
          case "drag":
            result = {
              ok: true,
              value: drag(command.match || command.id, {
                deltaX: command.delta_x,
                deltaY: command.delta_y,
                toX: command.to_x,
                toY: command.to_y,
                steps: command.steps,
                handle: command.handle
              })
            };
            break;
          case "geometry":
            result = { ok: true, value: geometry(command.match || command.id) };
            break;
          case "eval":
            result = { ok: true, value: Function(command.code)() };
            break;
          default:
            result = { ok: false, error: `unsupported command ${command.type}` };
            break;
        }
        const eventEnd = queue.length;
        if (result && result.value && typeof result.value === "object" && !Array.isArray(result.value)) {
          result.value._aegis = {
            command_index: index,
            event_span: {
              start: eventStart,
              end: eventEnd
            }
          };
        }
        return result;
      } catch (error) {
        return {
          ok: false,
          error: String(error && error.message ? error.message : error)
        };
      }
    });
  }

  function serializeMutation(mutation) {
    const changes = [];

    if (mutation.type === "attributes") {
      if (mutation.target && mutation.target.nodeType === Node.ELEMENT_NODE) {
        const serialized = serializeNode(mutation.target);
        if (serialized) {
          changes.push({ kind: "upsert", ...serialized });
        }
      }
      return changes;
    }

    if (mutation.type === "characterData") {
      const parent = mutation.target.parentElement;
      if (parent) {
        const serialized = serializeNode(parent);
        if (serialized) {
          changes.push({ kind: "upsert", ...serialized });
        }
      }
      return changes;
    }

    if (mutation.type === "childList") {
      for (const node of Array.from(mutation.addedNodes || [])) {
        collectSerializedSubtree(node, changes);
      }
      for (const node of Array.from(mutation.removedNodes || [])) {
        collectRemovedNodeIds(node, changes);
      }

      const target =
        mutation.target && mutation.target.nodeType === Node.ELEMENT_NODE
          ? mutation.target
          : mutation.target && mutation.target.parentElement
            ? mutation.target.parentElement
            : null;
      const id = assignId(target);
      if (id != null) {
        changes.push({
          kind: "set_children",
          id,
          children: Array.from(target.children || [])
            .map((child) => assignId(child))
            .filter((childId) => childId != null)
        });
      }
    }

    return changes;
  }

  function drainEvents() {
    return queue.splice(0, queue.length);
  }

  const observer = new MutationObserver((mutations) => {
    markDocumentLoaded();
    for (const mutation of mutations) {
      if (mutation.type === "childList") {
        for (const node of Array.from(mutation.addedNodes || [])) {
          instrumentMediaTree(node);
        }
      }
    }
    pageBootstrapState.dom_mutation_count += mutations.length;
    if (documentLoaded()) {
      const body = document.body || null;
      const root = rootCandidate();
      pageBootstrapState.dom_mutation_after_load_count += mutations.length;
      for (const mutation of mutations) {
        const target = mutation.target && mutation.target.nodeType === Node.ELEMENT_NODE
          ? mutation.target
          : mutation.target && mutation.target.parentElement
            ? mutation.target.parentElement
            : null;
        if (body && target && (target === body || body.contains(target))) {
          pageBootstrapState.body_mutation_after_load_count += 1;
        }
        if (root && target && (target === root || root.contains(target))) {
          pageBootstrapState.root_mutation_after_load_count += 1;
        }
      }
    }
    const changes = mutations.flatMap(serializeMutation);
    if (changes.length > 0) {
      queue.push({
        event: {
          type: "dom_mutation",
          changes
        }
      });
    }
  });

  let observerAttached = false;

  function attachObserver() {
    if (observerAttached || !document.documentElement) {
      return;
    }
    observerAttached = true;
    observer.observe(document.documentElement, {
      subtree: true,
      childList: true,
      characterData: true,
      attributes: true
    });
  }

  attachObserver();
  document.addEventListener("readystatechange", markDocumentLoaded);
  window.addEventListener("load", markDocumentLoaded, { once: true });
  window.addEventListener("error", (event) => {
    const target = event.target;
    const source = target && target.tagName
      ? `${target.tagName.toLowerCase()}:${target.getAttribute("src") || target.getAttribute("href") || ""}`
      : event.filename || "window";
    pageBootstrapState.script_error_count += 1;
    pageBootstrapState.last_script_error = `${source}:${event.message || "script error"}`;
    queue.push({
      event: {
        type: "log",
        level: "warn",
        message: "page:script_error",
        data: {
          source,
          message: event.message || null
        }
      }
    });
  }, true);
  window.addEventListener("unhandledrejection", (event) => {
    const reason = event && event.reason ? String(event.reason) : "unhandled rejection";
    pageBootstrapState.unhandled_rejection_count += 1;
    pageBootstrapState.last_unhandled_rejection = reason;
    queue.push({
      event: {
        type: "log",
        level: "warn",
        message: "page:unhandled_rejection",
        data: {
          reason
        }
      }
    });
  });
  patchMediaPrototype();
  instrumentMediaTree(document);
  if (!observerAttached) {
    document.addEventListener("DOMContentLoaded", attachObserver, { once: true });
  }

  window.__aegis_queue = queue;
  window.__aegis = {
    snapshot,
    exec,
    click,
    hover,
    pressKey,
    setValue,
    setFiles,
    scrollToPosition,
    drag,
    geometry,
    drainEvents,
    queue,
    assignId,
    currentPageState,
    resolveTargetInfo,
    waitState,
    pageResearchData
  };
})();
