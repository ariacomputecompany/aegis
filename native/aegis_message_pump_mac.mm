#import <Cocoa/Cocoa.h>

#include "aegis_native_mac.h"

#include <algorithm>
#include <chrono>
#include <mutex>

namespace {

struct AegisMessagePumpScheduler {
  std::mutex mutex;
  dispatch_source_t timer = nil;
  bool work_pending = false;
  std::chrono::steady_clock::time_point due_at = std::chrono::steady_clock::time_point::min();
};

AegisMessagePumpScheduler& MessagePumpScheduler() {
  static AegisMessagePumpScheduler scheduler;
  return scheduler;
}

void ConfigureMessagePumpTimerOnMainQueue() {
  auto& scheduler = MessagePumpScheduler();
  dispatch_assert_queue(dispatch_get_main_queue());

  std::chrono::steady_clock::time_point due_at;
  bool work_pending = false;
  {
    std::lock_guard lock(scheduler.mutex);
    work_pending = scheduler.work_pending;
    due_at = scheduler.due_at;
    if (scheduler.timer == nil) {
      scheduler.timer = dispatch_source_create(
          DISPATCH_SOURCE_TYPE_TIMER, 0, 0, dispatch_get_main_queue());
      dispatch_source_set_event_handler(scheduler.timer, ^{
        AegisRunScheduledCefMessagePumpWorkIfDue();
        ConfigureMessagePumpTimerOnMainQueue();
      });
      dispatch_resume(scheduler.timer);
    }
  }

  if (scheduler.timer == nil) {
    return;
  }

  if (!work_pending) {
    dispatch_source_set_timer(scheduler.timer, DISPATCH_TIME_FOREVER,
                              DISPATCH_TIME_FOREVER, 0);
    return;
  }

  const auto now = std::chrono::steady_clock::now();
  const auto delay = due_at > now
                         ? std::chrono::duration_cast<std::chrono::nanoseconds>(due_at - now)
                         : std::chrono::nanoseconds::zero();
  dispatch_source_set_timer(scheduler.timer,
                            dispatch_time(DISPATCH_TIME_NOW, delay.count()),
                            DISPATCH_TIME_FOREVER, 0);
}

void DispatchToMainQueue(void (^block)(void)) {
  if ([NSThread isMainThread]) {
    block();
    return;
  }
  dispatch_async(dispatch_get_main_queue(), block);
}

}  // namespace

void AegisScheduleCefMessagePumpWork(int64_t delay_ms) {
  auto& scheduler = MessagePumpScheduler();
  const auto clamped_delay = std::max<int64_t>(delay_ms, 0);
  {
    std::lock_guard lock(scheduler.mutex);
    scheduler.work_pending = true;
    scheduler.due_at =
        std::chrono::steady_clock::now() + std::chrono::milliseconds(clamped_delay);
  }
  DispatchToMainQueue(^{
    ConfigureMessagePumpTimerOnMainQueue();
  });
}

bool AegisRunScheduledCefMessagePumpWorkIfDue() {
  auto& scheduler = MessagePumpScheduler();
  {
    std::lock_guard lock(scheduler.mutex);
    if (!scheduler.work_pending ||
        std::chrono::steady_clock::now() < scheduler.due_at) {
      return false;
    }
    scheduler.work_pending = false;
  }

  CefDoMessageLoopWork();
  return true;
}

int64_t AegisNextScheduledCefWorkDelayMs() {
  auto& scheduler = MessagePumpScheduler();
  std::lock_guard lock(scheduler.mutex);
  if (!scheduler.work_pending) {
    return -1;
  }
  const auto now = std::chrono::steady_clock::now();
  if (scheduler.due_at <= now) {
    return 0;
  }
  return std::chrono::duration_cast<std::chrono::milliseconds>(scheduler.due_at - now).count();
}

void AegisResetCefMessagePumpScheduler() {
  auto& scheduler = MessagePumpScheduler();
  {
    std::lock_guard lock(scheduler.mutex);
    scheduler.work_pending = false;
    scheduler.due_at = std::chrono::steady_clock::time_point::min();
  }
  DispatchToMainQueue(^{
    auto& state = MessagePumpScheduler();
    dispatch_source_t timer = nil;
    {
      std::lock_guard lock(state.mutex);
      timer = state.timer;
      state.timer = nil;
    }
    if (timer != nil) {
      dispatch_source_cancel(timer);
    }
  });
}

void AegisRunApplicationMessageLoop() {
  AegisScheduleCefMessagePumpWork(0);
  [NSApp run];
}

void AegisStopApplicationMessageLoop() {
  DispatchToMainQueue(^{
    [NSApp stop:nil];
    [NSApp postEvent:[NSEvent otherEventWithType:NSEventTypeApplicationDefined
                                        location:NSZeroPoint
                                   modifierFlags:0
                                       timestamp:0
                                    windowNumber:0
                                         context:nil
                                         subtype:0
                                           data1:0
                                           data2:0]
             atStart:NO];
  });
}
