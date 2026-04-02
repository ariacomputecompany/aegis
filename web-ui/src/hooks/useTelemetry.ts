import { startTransition, useEffect, useState } from "react";

export interface TelemetryResponse {
  diagnostics: {
    state: string;
    command_ready: boolean;
    bridge_healthy: boolean;
    browser_backend_healthy: boolean;
    total_operations: number;
    successful_operations: number;
    timed_out_operations: number;
    recent_operations: Array<{
      id: number;
      name: string;
      stage: string;
      status: string;
      started_at_ms: number;
      finished_at_ms: number;
      elapsed_ms: number;
      timed_out: boolean;
      error_message?: string | null;
    }>;
    operation_aggregates: Array<{
      name: string;
      total_count: number;
      success_count: number;
      failure_count: number;
      timeout_count: number;
      avg_elapsed_ms: number;
      min_elapsed_ms: number;
      max_elapsed_ms: number;
      last_elapsed_ms: number;
    }>;
  };
  chrome: {
    title: string;
    url: string;
    can_go_back: boolean;
    can_go_forward: boolean;
    is_loading: boolean;
  };
  runtime: {
    status: {
      current_url?: string | null;
      current_title?: string | null;
      document_ready_state?: string | null;
      dom_nodes: number;
      retained_event_count: number;
      latest_event_sequence: number;
      last_dom_refresh_at_ms?: number | null;
      last_live_state_refresh_at_ms?: number | null;
      last_event_at_ms?: number | null;
      last_successful_command_at_ms?: number | null;
      bootstrap_duration_ms?: number | null;
    };
    dom: {
      total_nodes: number;
      actionable_nodes: number;
      visible_nodes: number;
      disabled_nodes: number;
      text_nodes: number;
    };
    events: {
      total_events: number;
      dom_mutation_events: number;
      dom_mutation_changes: number;
      navigation_events: number;
      network_events: number;
      log_events: number;
      recent_navigations: Array<{
        sequence: number;
        timestamp_ms: number;
        url: string;
      }>;
      recent_network_requests: Array<{
        sequence: number;
        timestamp_ms: number;
        request_id: string;
        url: string;
      }>;
      recent_logs: Array<{
        sequence: number;
        timestamp_ms: number;
        level: string;
        message: string;
      }>;
    };
    page?: {
      sampled_at_ms: number;
      url?: string | null;
      title?: string | null;
      ready_state?: string | null;
      origin?: string | null;
      visibility_state?: string | null;
      has_focus?: boolean | null;
      viewport: {
        width?: number | null;
        height?: number | null;
        device_pixel_ratio?: number | null;
        scroll_x?: number | null;
        scroll_y?: number | null;
      };
      navigation: {
        navigation_type?: string | null;
        dom_content_loaded_ms?: number | null;
        load_event_ms?: number | null;
        dom_interactive_ms?: number | null;
        response_end_ms?: number | null;
        transfer_size_bytes?: number | null;
        encoded_body_size_bytes?: number | null;
        decoded_body_size_bytes?: number | null;
      };
      resources: {
        resource_count: number;
        script_count: number;
        stylesheet_count: number;
        image_count: number;
        fetch_count: number;
        xml_http_request_count: number;
        other_count: number;
        transfer_size_bytes?: number | null;
      };
      js_heap: {
        used_js_heap_size_bytes?: number | null;
        total_js_heap_size_bytes?: number | null;
        js_heap_size_limit_bytes?: number | null;
      };
      paint: {
        first_paint_ms?: number | null;
        first_contentful_paint_ms?: number | null;
        largest_contentful_paint_ms?: number | null;
        largest_contentful_paint_size?: number | null;
      };
      stability: {
        cumulative_layout_shift?: number | null;
        layout_shift_count: number;
      };
      responsiveness: {
        long_task_count: number;
        long_task_total_duration_ms?: number | null;
        long_task_max_duration_ms?: number | null;
        event_count: number;
        interaction_count: number;
        total_event_duration_ms?: number | null;
        max_event_duration_ms?: number | null;
        first_input_delay_ms?: number | null;
      };
    } | null;
    page_capture_error?: string | null;
    trace: {
      enabled: boolean;
      path?: string | null;
      recorded_batches: number;
      initial_session_captured: boolean;
      file_size_bytes?: number | null;
    };
  };
  session: {
    profile: {
      profile: string;
      path: string;
    };
    cookie_count: number;
    local_storage_count: number;
    session_storage_count: number;
    network_override_count: number;
    cookies: Array<{
      name: string;
      domain: string;
      path?: string | null;
      expires_unix?: number | null;
      secure: boolean;
      http_only: boolean;
      value_bytes: number;
    }>;
    local_storage: Array<{ key: string; value_bytes: number }>;
    session_storage: Array<{ key: string; value_bytes: number }>;
    network_overrides: Array<{ header: string; value_bytes: number }>;
  };
  credentials: {
    settings: {
      auto_store: boolean;
    };
    stored_credentials_count: number;
    entries: Array<{
      origin: string;
      username: string;
      username_field?: string | null;
      password_field?: string | null;
      form_label?: string | null;
      created_at_ms: number;
      updated_at_ms: number;
    }>;
  };
  settings: {
    default_profile?: string | null;
    headless_persistent?: boolean | null;
    headful_persistent?: boolean | null;
  };
  dashboard: {
    headful_dashboard: boolean;
    resolution?: string | null;
    vnc_addr?: string | null;
  };
}

interface TelemetryState {
  data: TelemetryResponse | null;
  error: string | null;
  loading: boolean;
}

const INITIAL_STATE: TelemetryState = {
  data: null,
  error: null,
  loading: true,
};

export function useTelemetry(): TelemetryState {
  const [state, setState] = useState<TelemetryState>(INITIAL_STATE);

  useEffect(() => {
    let cancelled = false;

    async function load() {
      try {
        const response = await fetch("/telemetry");
        if (!response.ok) {
          throw new Error(`telemetry failed with ${response.status}`);
        }
        const data = (await response.json()) as TelemetryResponse;
        if (cancelled) {
          return;
        }
        startTransition(() => {
          setState({
            data,
            error: null,
            loading: false,
          });
        });
      } catch (error) {
        if (cancelled) {
          return;
        }
        startTransition(() => {
          setState((current) => ({
            data: current.data,
            error:
              error instanceof Error
                ? error.message
                : "failed to load telemetry",
            loading: false,
          }));
        });
      }
    }

    void load();
    const interval = window.setInterval(() => {
      void load();
    }, 2500);

    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, []);

  return state;
}
