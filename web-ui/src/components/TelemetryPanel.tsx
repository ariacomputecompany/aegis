import { useTelemetry } from "../hooks/useTelemetry";

function formatBytes(value?: number | null): string {
  if (value == null || Number.isNaN(value)) {
    return "n/a";
  }
  if (value < 1024) {
    return `${value} B`;
  }
  if (value < 1024 * 1024) {
    return `${(value / 1024).toFixed(1)} KB`;
  }
  if (value < 1024 * 1024 * 1024) {
    return `${(value / (1024 * 1024)).toFixed(1)} MB`;
  }
  return `${(value / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function formatMs(value?: number | null): string {
  if (value == null || Number.isNaN(value)) {
    return "n/a";
  }
  return `${Math.round(value)} ms`;
}

function formatTime(value?: number | null): string {
  if (value == null || Number.isNaN(value)) {
    return "n/a";
  }
  return new Date(value).toLocaleTimeString();
}

function boolLabel(value?: boolean | null): string {
  if (value == null) {
    return "n/a";
  }
  return value ? "yes" : "no";
}

export function TelemetryPanel() {
  const { data, error, loading } = useTelemetry();

  return (
    <aside className="telemetry-panel">
      <div className="telemetry-panel__header">
        <div>
          <p className="telemetry-panel__eyebrow">Aegis Telemetry</p>
          <h2 className="telemetry-panel__title">Production Session Surface</h2>
        </div>
        <div className="telemetry-panel__badge" data-state={data?.diagnostics.state ?? "idle"}>
          {data?.diagnostics.state ?? (loading ? "loading" : "idle")}
        </div>
      </div>

      {error ? <div className="telemetry-panel__error">{error}</div> : null}

      <div className="telemetry-panel__scroll">
        {data ? (
          <>
            <section className="telemetry-section">
              <h3>Runtime</h3>
              <div className="telemetry-grid">
                <Metric label="Command ready" value={boolLabel(data.diagnostics.command_ready)} />
                <Metric label="Bridge healthy" value={boolLabel(data.diagnostics.bridge_healthy)} />
                <Metric
                  label="Browser healthy"
                  value={boolLabel(data.diagnostics.browser_backend_healthy)}
                />
                <Metric
                  label="Bootstrap"
                  value={formatMs(data.runtime.status.bootstrap_duration_ms)}
                />
                <Metric label="Profile" value={data.session.profile.profile} />
                <Metric label="Default profile" value={data.settings.default_profile ?? "n/a"} />
              </div>
            </section>

            <section className="telemetry-section">
              <h3>Page</h3>
              <div className="telemetry-grid">
                <Metric label="URL" value={data.runtime.page?.url ?? (data.chrome.url || "n/a")} />
                <Metric label="Title" value={data.runtime.page?.title ?? (data.chrome.title || "n/a")} />
                <Metric
                  label="Ready state"
                  value={data.runtime.page?.ready_state ?? data.runtime.status.document_ready_state ?? "n/a"}
                />
                <Metric label="Visibility" value={data.runtime.page?.visibility_state ?? "n/a"} />
                <Metric label="Has focus" value={boolLabel(data.runtime.page?.has_focus)} />
                <Metric
                  label="Viewport"
                  value={
                    data.runtime.page?.viewport.width && data.runtime.page?.viewport.height
                      ? `${data.runtime.page.viewport.width} x ${data.runtime.page.viewport.height}`
                      : "n/a"
                  }
                />
                <Metric
                  label="DOMContentLoaded"
                  value={formatMs(data.runtime.page?.navigation.dom_content_loaded_ms)}
                />
                <Metric
                  label="Load event"
                  value={formatMs(data.runtime.page?.navigation.load_event_ms)}
                />
                <Metric
                  label="Transfer size"
                  value={formatBytes(data.runtime.page?.navigation.transfer_size_bytes)}
                />
                <Metric
                  label="Resources"
                  value={String(data.runtime.page?.resources.resource_count ?? 0)}
                />
                <Metric
                  label="JS heap used"
                  value={formatBytes(data.runtime.page?.js_heap.used_js_heap_size_bytes)}
                />
                <Metric
                  label="Telemetry sample"
                  value={formatTime(data.runtime.page?.sampled_at_ms)}
                />
              </div>
              <div className="telemetry-grid telemetry-grid--spaced">
                <Metric
                  label="First paint"
                  value={formatMs(data.runtime.page?.paint.first_paint_ms)}
                />
                <Metric
                  label="FCP"
                  value={formatMs(data.runtime.page?.paint.first_contentful_paint_ms)}
                />
                <Metric
                  label="LCP"
                  value={formatMs(data.runtime.page?.paint.largest_contentful_paint_ms)}
                />
                <Metric
                  label="LCP size"
                  value={formatBytes(data.runtime.page?.paint.largest_contentful_paint_size)}
                />
                <Metric
                  label="CLS"
                  value={
                    data.runtime.page?.stability.cumulative_layout_shift != null
                      ? data.runtime.page.stability.cumulative_layout_shift.toFixed(3)
                      : "n/a"
                  }
                />
                <Metric
                  label="Layout shifts"
                  value={String(data.runtime.page?.stability.layout_shift_count ?? 0)}
                />
                <Metric
                  label="Long tasks"
                  value={String(data.runtime.page?.responsiveness.long_task_count ?? 0)}
                />
                <Metric
                  label="Worst long task"
                  value={formatMs(data.runtime.page?.responsiveness.long_task_max_duration_ms)}
                />
                <Metric
                  label="Interactions"
                  value={String(data.runtime.page?.responsiveness.interaction_count ?? 0)}
                />
                <Metric
                  label="First input delay"
                  value={formatMs(data.runtime.page?.responsiveness.first_input_delay_ms)}
                />
              </div>
              {data.runtime.page_capture_error ? (
                <p className="telemetry-section__note">{data.runtime.page_capture_error}</p>
              ) : null}
            </section>

            <section className="telemetry-section">
              <h3>DOM And Events</h3>
              <div className="telemetry-grid">
                <Metric label="DOM nodes" value={String(data.runtime.dom.total_nodes)} />
                <Metric label="Actionable nodes" value={String(data.runtime.dom.actionable_nodes)} />
                <Metric label="Visible nodes" value={String(data.runtime.dom.visible_nodes)} />
                <Metric label="Disabled nodes" value={String(data.runtime.dom.disabled_nodes)} />
                <Metric label="Text nodes" value={String(data.runtime.dom.text_nodes)} />
                <Metric label="Total events" value={String(data.runtime.events.total_events)} />
                <Metric
                  label="DOM mutations"
                  value={`${data.runtime.events.dom_mutation_events} / ${data.runtime.events.dom_mutation_changes} changes`}
                />
                <Metric
                  label="Network events"
                  value={String(data.runtime.events.network_events)}
                />
                <Metric
                  label="Navigation events"
                  value={String(data.runtime.events.navigation_events)}
                />
                <Metric label="Log events" value={String(data.runtime.events.log_events)} />
                <Metric
                  label="Latest sequence"
                  value={String(data.runtime.status.latest_event_sequence)}
                />
                <Metric
                  label="Retained events"
                  value={String(data.runtime.status.retained_event_count)}
                />
              </div>
              <TelemetryList
                title="Recent navigations"
                items={data.runtime.events.recent_navigations.map((item) => ({
                  key: `${item.sequence}`,
                  primary: item.url,
                  secondary: `seq ${item.sequence} at ${formatTime(item.timestamp_ms)}`,
                }))}
              />
              <TelemetryList
                title="Recent network requests"
                items={data.runtime.events.recent_network_requests.map((item) => ({
                  key: item.request_id,
                  primary: `${item.method ?? "request"} ${item.url}`,
                  secondary:
                    `${item.request_id} at ${formatTime(item.timestamp_ms)}` +
                    `${item.status_code != null ? ` • ${item.status_code}` : ""}` +
                    `${item.request_status ? ` • ${item.request_status}` : ""}` +
                    `${item.duration_ms != null ? ` • ${formatMs(item.duration_ms)}` : ""}` +
                    `${item.redirect_url ? ` • redirect ${item.redirect_url}` : ""}` +
                    `${item.error_text ? ` • ${item.error_text}` : ""}` +
                    `${item.received_content_length_bytes != null ? ` • rx ${formatBytes(item.received_content_length_bytes)}` : ""}` +
                    `${item.content_length_bytes != null ? ` • ${formatBytes(item.content_length_bytes)}` : ""}` +
                    `${item.response_headers ? ` • ${Object.keys(item.response_headers).length} headers` : ""}`,
                }))}
              />
              <TelemetryList
                title="Recent runtime logs"
                items={data.runtime.events.recent_logs.map((item, index) => ({
                  key: `${item.sequence}-${index}`,
                  primary: `[${item.level}] ${item.message}`,
                  secondary: `seq ${item.sequence} at ${formatTime(item.timestamp_ms)}`,
                }))}
              />
            </section>

            <section className="telemetry-section">
              <h3>Session And Credentials</h3>
              <div className="telemetry-grid">
                <Metric label="Cookies" value={String(data.session.cookie_count)} />
                <Metric
                  label="Local storage keys"
                  value={String(data.session.local_storage_count)}
                />
                <Metric
                  label="Session storage keys"
                  value={String(data.session.session_storage_count)}
                />
                <Metric
                  label="Network overrides"
                  value={String(data.session.network_override_count)}
                />
                <Metric
                  label="Stored credentials"
                  value={String(data.credentials.stored_credentials_count)}
                />
                <Metric
                  label="Auto-store credentials"
                  value={boolLabel(data.credentials.settings.auto_store)}
                />
                <Metric
                  label="Headless persistent"
                  value={boolLabel(data.settings.headless_persistent)}
                />
                <Metric
                  label="Headful persistent"
                  value={boolLabel(data.settings.headful_persistent)}
                />
              </div>
              <TelemetryList
                title="Cookie inventory"
                items={data.session.cookies.map((item) => ({
                  key: `${item.domain}-${item.name}`,
                  primary: `${item.name} @ ${item.domain}`,
                  secondary: `${formatBytes(item.value_bytes)} • secure=${boolLabel(item.secure)} • http_only=${boolLabel(item.http_only)}`,
                }))}
              />
              <TelemetryList
                title="Stored credentials"
                items={data.credentials.entries.map((item) => ({
                  key: `${item.origin}-${item.username}`,
                  primary: `${item.username} @ ${item.origin}`,
                  secondary: `updated ${formatTime(item.updated_at_ms)}${item.form_label ? ` • ${item.form_label}` : ""}`,
                }))}
              />
            </section>

            <section className="telemetry-section">
              <h3>Tracing And Operations</h3>
              <div className="telemetry-grid">
                <Metric label="Trace enabled" value={boolLabel(data.runtime.trace.enabled)} />
                <Metric
                  label="Recorded batches"
                  value={String(data.runtime.trace.recorded_batches)}
                />
                <Metric
                  label="Initial session captured"
                  value={boolLabel(data.runtime.trace.initial_session_captured)}
                />
                <Metric
                  label="Trace file size"
                  value={formatBytes(data.runtime.trace.file_size_bytes)}
                />
                <Metric
                  label="Total operations"
                  value={String(data.diagnostics.total_operations)}
                />
                <Metric
                  label="Successful operations"
                  value={String(data.diagnostics.successful_operations)}
                />
                <Metric
                  label="Timed out operations"
                  value={String(data.diagnostics.timed_out_operations)}
                />
                <Metric
                  label="Dashboard resolution"
                  value={data.dashboard.resolution ?? "n/a"}
                />
              </div>
              <TelemetryList
                title="Operation aggregates"
                items={data.diagnostics.operation_aggregates.map((item) => ({
                  key: item.name,
                  primary: `${item.name}: avg ${formatMs(item.avg_elapsed_ms)} / max ${formatMs(item.max_elapsed_ms)}`,
                  secondary: `${item.success_count} ok • ${item.failure_count} failed • ${item.timeout_count} timed out`,
                }))}
              />
              <TelemetryList
                title="Recent operations"
                items={data.diagnostics.recent_operations.map((item) => ({
                  key: `${item.id}`,
                  primary: `${item.name} (${item.status})`,
                  secondary: `${item.stage} • ${formatMs(item.elapsed_ms)} • ${formatTime(item.finished_at_ms)}`,
                }))}
              />
            </section>
          </>
        ) : (
          <div className="telemetry-panel__placeholder">
            {loading ? "Loading production telemetry" : "Telemetry unavailable"}
          </div>
        )}
      </div>
    </aside>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="telemetry-metric">
      <span className="telemetry-metric__label">{label}</span>
      <span className="telemetry-metric__value">{value}</span>
    </div>
  );
}

function TelemetryList({
  title,
  items,
}: {
  title: string;
  items: Array<{ key: string; primary: string; secondary: string }>;
}) {
  return (
    <div className="telemetry-list">
      <p className="telemetry-list__title">{title}</p>
      {items.length === 0 ? (
        <p className="telemetry-list__empty">No data yet</p>
      ) : (
        items.slice(0, 8).map((item) => (
          <div className="telemetry-list__item" key={item.key}>
            <p className="telemetry-list__primary">{item.primary}</p>
            <p className="telemetry-list__secondary">{item.secondary}</p>
          </div>
        ))
      )}
    </div>
  );
}
