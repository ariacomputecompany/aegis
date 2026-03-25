// Shared Cocoa browser host window implementation for both the standalone app
// and the persistent host dylib.

// ═══════════════════════════════════════════════════════════════════════════════
// DESIGN SYSTEM — Vercel/shadcn-inspired browser chrome
// ═══════════════════════════════════════════════════════════════════════════════

static const CGFloat kToolbarHeight = 82.0;
static const CGFloat kTabHeight = 28.0;
static const CGFloat kTabRadius = 8.0;
static const CGFloat kTabLeftInset = 76.0;
static const CGFloat kTabHPad = 12.0;
static const CGFloat kTabInitialWidth = 200.0;
static const CGFloat kNavBtnSize = 28.0;
static const CGFloat kNavBtnRadius = 6.0;
static const CGFloat kNavBtnGap = 4.0;
static const CGFloat kNavLeftInset = 14.0;
static const CGFloat kAddrHeight = 32.0;
static const CGFloat kAddrRadius = 8.0;
static const CGFloat kAddrHPad = 10.0;
static const CGFloat kAddrGapFromNav = 10.0;
static const CGFloat kAddrRightInset = 14.0;
static const CGFloat kProgressHeight = 2.0;
static const CGFloat kSeparatorHeight = 1.0;
static const CGFloat kPageTransitionMaxAlpha = 0.16;
static const CGFloat kPageTransitionDelay = 0.05;
static const CGFloat kPageTransitionFadeInDuration = 0.14;
static const CGFloat kPageTransitionFadeOutDuration = 0.18;
static const CGFloat kPageTransitionMinVisibleDuration = 0.12;

static NSColor* AegisColor(CGFloat r, CGFloat g, CGFloat b, CGFloat a) {
  return [NSColor colorWithSRGBRed:r green:g blue:b alpha:a];
}

static NSColor* AegisWindowBg(void) { return AegisColor(0.975, 0.975, 0.975, 1.0); }
static NSColor* AegisActiveTabBorder(void) { return AegisColor(0.0, 0.0, 0.0, 0.08); }
static NSColor* AegisPrimaryText(void) { return AegisColor(0.10, 0.10, 0.10, 1.0); }
static NSColor* AegisSecondaryText(void) { return AegisColor(0.0, 0.0, 0.0, 0.50); }
static NSColor* AegisPlaceholderText(void) { return AegisColor(0.0, 0.0, 0.0, 0.32); }
static NSColor* AegisAddrBg(void) { return AegisColor(0.0, 0.0, 0.0, 0.055); }
static NSColor* AegisAddrHoverBg(void) { return AegisColor(0.0, 0.0, 0.0, 0.075); }
static NSColor* AegisAddrFocusBg(void) { return [NSColor whiteColor]; }
static NSColor* AegisAddrBorder(void) { return AegisColor(0.0, 0.0, 0.0, 0.09); }
static NSColor* AegisAddrFocusBorder(void) { return AegisColor(0.23, 0.51, 0.97, 0.50); }
static NSColor* AegisBtnHoverBg(void) { return AegisColor(0.0, 0.0, 0.0, 0.06); }
static NSColor* AegisBtnPressedBg(void) { return AegisColor(0.0, 0.0, 0.0, 0.10); }
static NSColor* AegisSeparator(void) { return AegisColor(0.0, 0.0, 0.0, 0.12); }
static NSColor* AegisAccent(void) { return AegisColor(0.23, 0.51, 0.97, 1.0); }
static NSColor* AegisPageTransitionFill(void) { return AegisColor(0.972, 0.976, 0.985, 1.0); }
static NSColor* AegisNavIconDefault(void) { return AegisColor(0.25, 0.25, 0.25, 1.0); }
static NSColor* AegisNavIconActive(void) { return AegisColor(0.12, 0.12, 0.12, 1.0); }
static NSColor* AegisLockIcon(void) { return AegisColor(0.0, 0.0, 0.0, 0.36); }

@interface AegisHostApplication : NSApplication <CefAppProtocol> {
 @private
  BOOL handlingSendEvent_;
}
@end

@implementation AegisHostApplication
- (BOOL)isHandlingSendEvent {
  return handlingSendEvent_;
}

- (void)setHandlingSendEvent:(BOOL)handlingSendEvent {
  handlingSendEvent_ = handlingSendEvent;
}

- (void)sendEvent:(NSEvent*)event {
  CefScopedSendingEvent sendingEventScoper;
  [super sendEvent:event];
}
@end

@interface AegisNavButton : NSButton
- (instancetype)initWithSymbolName:(NSString*)name;
- (void)setSymbolName:(NSString*)name;
@end

@implementation AegisNavButton {
  NSTrackingArea* _trackingArea;
  BOOL _hovered;
}

- (instancetype)initWithSymbolName:(NSString*)name {
  self = [super initWithFrame:NSMakeRect(0, 0, kNavBtnSize, kNavBtnSize)];
  if (self) {
    [self setButtonType:NSButtonTypeMomentaryChange];
    [self setBezelStyle:NSBezelStyleInline];
    [self setBordered:NO];
    [self setTitle:@""];
    [self setImagePosition:NSImageOnly];
    [self setImageScaling:NSImageScaleProportionallyDown];
    [self setWantsLayer:YES];
    self.layer.cornerRadius = kNavBtnRadius;
    [self setSymbolName:name];
  }
  return self;
}

- (void)setSymbolName:(NSString*)name {
  NSImageSymbolConfiguration* config = [NSImageSymbolConfiguration
      configurationWithPointSize:13.0
                          weight:NSFontWeightMedium
                           scale:NSImageSymbolScaleMedium];
  NSImage* image = [NSImage imageWithSystemSymbolName:name
                                 accessibilityDescription:name];
  if (image) {
    image = [image imageWithSymbolConfiguration:config];
    [image setTemplate:YES];
    [self setImage:image];
    [self setTitle:@""];
    [self setImagePosition:NSImageOnly];
  }
  [self setContentTintColor:AegisNavIconDefault()];
}

- (void)setEnabled:(BOOL)enabled {
  [super setEnabled:enabled];
  self.alphaValue = enabled ? 1.0 : 0.30;
  if (!enabled) {
    _hovered = NO;
    self.layer.backgroundColor = nil;
  }
}

- (void)updateTrackingAreas {
  [super updateTrackingAreas];
  if (_trackingArea) {
    [self removeTrackingArea:_trackingArea];
  }
  _trackingArea = [[NSTrackingArea alloc]
      initWithRect:self.bounds
           options:(NSTrackingMouseEnteredAndExited | NSTrackingActiveInActiveApp)
             owner:self
          userInfo:nil];
  [self addTrackingArea:_trackingArea];
}

- (void)mouseEntered:(NSEvent*)event {
  (void)event;
  if (!self.isEnabled) return;
  _hovered = YES;
  self.layer.backgroundColor = AegisBtnHoverBg().CGColor;
  [self setContentTintColor:AegisNavIconActive()];
}

- (void)mouseExited:(NSEvent*)event {
  (void)event;
  _hovered = NO;
  self.layer.backgroundColor = nil;
  [self setContentTintColor:AegisNavIconDefault()];
}

- (void)mouseDown:(NSEvent*)event {
  if (!self.isEnabled) return;
  self.layer.backgroundColor = AegisBtnPressedBg().CGColor;
  [super mouseDown:event];
  NSPoint loc = [self convertPoint:[self.window mouseLocationOutsideOfEventStream]
                          fromView:nil];
  if (NSPointInRect(loc, self.bounds)) {
    _hovered = YES;
    self.layer.backgroundColor = AegisBtnHoverBg().CGColor;
  } else {
    _hovered = NO;
    self.layer.backgroundColor = nil;
  }
  [self setContentTintColor:AegisNavIconDefault()];
}

@end

@interface AegisAddressContainer : NSView
@property (nonatomic, assign) BOOL focused;
@end

@implementation AegisAddressContainer {
  NSTrackingArea* _trackingArea;
  BOOL _hovered;
  BOOL _focused;
}

@synthesize focused = _focused;

- (instancetype)initWithFrame:(NSRect)frame {
  self = [super initWithFrame:frame];
  if (self) {
    [self setWantsLayer:YES];
    self.layer.cornerRadius = kAddrRadius;
    self.layer.borderWidth = 1.0;
    self.layer.masksToBounds = YES;
    [self applyDefaultStyle];
  }
  return self;
}

- (void)applyDefaultStyle {
  self.layer.backgroundColor = AegisAddrBg().CGColor;
  self.layer.borderColor = AegisAddrBorder().CGColor;
  self.layer.borderWidth = 1.0;
}

- (void)applyHoverStyle {
  self.layer.backgroundColor = AegisAddrHoverBg().CGColor;
  self.layer.borderColor = AegisAddrBorder().CGColor;
  self.layer.borderWidth = 1.0;
}

- (void)applyFocusedStyle {
  self.layer.backgroundColor = AegisAddrFocusBg().CGColor;
  self.layer.borderColor = AegisAddrFocusBorder().CGColor;
  self.layer.borderWidth = 1.5;
}

- (void)refreshStyle {
  if (_focused) {
    [self applyFocusedStyle];
  } else if (_hovered) {
    [self applyHoverStyle];
  } else {
    [self applyDefaultStyle];
  }
}

- (void)setFocused:(BOOL)focused {
  _focused = focused;
  [self refreshStyle];
}

- (void)updateTrackingAreas {
  [super updateTrackingAreas];
  if (_trackingArea) {
    [self removeTrackingArea:_trackingArea];
  }
  _trackingArea = [[NSTrackingArea alloc]
      initWithRect:self.bounds
           options:(NSTrackingMouseEnteredAndExited | NSTrackingActiveInActiveApp)
             owner:self
          userInfo:nil];
  [self addTrackingArea:_trackingArea];
}

- (void)mouseEntered:(NSEvent*)event {
  (void)event;
  _hovered = YES;
  [self refreshStyle];
}

- (void)mouseExited:(NSEvent*)event {
  (void)event;
  _hovered = NO;
  [self refreshStyle];
}

@end

@interface AegisAddressField : NSTextField
@property (nonatomic, assign) AegisAddressContainer* addressContainer;
@end

@implementation AegisAddressField {
  AegisAddressContainer* _addressContainer;
}

@synthesize addressContainer = _addressContainer;

- (BOOL)becomeFirstResponder {
  BOOL result = [super becomeFirstResponder];
  if (result && self.addressContainer) {
    [self.addressContainer setFocused:YES];
    dispatch_async(dispatch_get_main_queue(), ^{
      NSText* editor = [[self window] fieldEditor:NO forObject:self];
      if (editor) {
        [editor selectAll:nil];
      }
    });
  }
  return result;
}

@end

@interface AegisBrowserChromeView : NSView
@end

@implementation AegisBrowserChromeView
- (BOOL)isOpaque {
  return YES;
}

- (void)drawRect:(NSRect)dirtyRect {
  [AegisWindowBg() setFill];
  NSRectFill(dirtyRect);
}
@end

@interface AegisBrowserWindowController : NSObject <NSWindowDelegate, NSTextFieldDelegate>
- (instancetype)initWithStatePointer:(void*)state;
- (void)navigateBack:(id)sender;
- (void)navigateForward:(id)sender;
- (void)reloadPage:(id)sender;
- (void)submitLocation:(id)sender;
- (void)updateAddress:(NSString*)address;
- (void)updateTitle:(NSString*)title;
- (void)updateNavigationButtonsWithCanGoBack:(BOOL)canGoBack
                                 canGoForward:(BOOL)canGoForward
                                    isLoading:(BOOL)isLoading;
- (void)unfocusAddressBar;
@end

namespace {

struct BrowserHostState {
  NSWindow* window = nil;
  NSView* root_view = nil;
  NSVisualEffectView* toolbar_view = nil;
  NSView* separator_view = nil;
  NSView* tab_bg = nil;
  NSTextField* tab_title_label = nil;
  AegisNavButton* new_tab_button = nil;
  AegisNavButton* back_button = nil;
  AegisNavButton* forward_button = nil;
  AegisNavButton* reload_button = nil;
  AegisAddressContainer* address_container = nil;
  NSImageView* lock_icon = nil;
  AegisAddressField* address_field = nil;
  NSView* progress_view = nil;
  NSView* web_container_view = nil;
  NSView* page_transition_view = nil;
  AegisBrowserWindowController* controller = nil;
  CefRefPtr<CefBrowser> browser;
  std::string current_title = "Aegis";
  std::string current_url;
  bool can_go_back = false;
  bool can_go_forward = false;
  bool is_loading = false;
  bool closing_from_browser = false;
  bool close_requested_by_user = false;
  CFTimeInterval loading_started_at = 0;
  CFTimeInterval transition_visible_since = 0;
  std::uint64_t transition_generation = 0;
};

BrowserHostState& HostState() {
  static BrowserHostState state;
  return state;
}

void InitializeHostApplication() {
  if (NSApp == nil) {
    [AegisHostApplication sharedApplication];
  }
  [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
  [NSApp finishLaunching];
}

void EnsureHostWindow(const std::string& title, int width, int height) {
  auto& s = HostState();
  if (s.window != nil && s.web_container_view != nil) {
    return;
  }

  InitializeHostApplication();

  const CGFloat w = static_cast<CGFloat>(width);
  const CGFloat h = static_cast<CGFloat>(height);

  s.window = [[NSWindow alloc]
      initWithContentRect:NSMakeRect(0, 0, w, h)
                styleMask:(NSWindowStyleMaskTitled |
                           NSWindowStyleMaskClosable |
                           NSWindowStyleMaskMiniaturizable |
                           NSWindowStyleMaskResizable |
                           NSWindowStyleMaskFullSizeContentView)
                  backing:NSBackingStoreBuffered
                    defer:NO];
  if (s.window == nil) {
    throw std::runtime_error("failed to create browser host window");
  }

  s.current_title = title;
  s.current_url = title;
  s.close_requested_by_user = false;
  [s.window setTitlebarAppearsTransparent:YES];
  [s.window setTitleVisibility:NSWindowTitleHidden];
  [s.window setMovableByWindowBackground:YES];
  [s.window setReleasedWhenClosed:NO];
  [s.window setBackgroundColor:AegisWindowBg()];
  [s.window setMinSize:NSMakeSize(520, 400)];
  [s.window center];

  s.root_view = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, w, h)];
  [s.root_view setAutoresizingMask:NSViewWidthSizable | NSViewHeightSizable];
  [s.window setContentView:s.root_view];

  s.toolbar_view = [[NSVisualEffectView alloc]
      initWithFrame:NSMakeRect(0, h - kToolbarHeight, w, kToolbarHeight)];
  [s.toolbar_view setMaterial:NSVisualEffectMaterialTitlebar];
  [s.toolbar_view setBlendingMode:NSVisualEffectBlendingModeWithinWindow];
  [s.toolbar_view setState:NSVisualEffectStateActive];
  [s.toolbar_view setEmphasized:YES];
  [s.toolbar_view setAppearance:[NSAppearance appearanceNamed:NSAppearanceNameAqua]];
  [s.toolbar_view setAutoresizingMask:NSViewWidthSizable | NSViewMinYMargin];
  [s.root_view addSubview:s.toolbar_view];

  s.separator_view = [[NSView alloc]
      initWithFrame:NSMakeRect(0, 0, w, kSeparatorHeight)];
  [s.separator_view setWantsLayer:YES];
  s.separator_view.layer.backgroundColor = AegisSeparator().CGColor;
  [s.separator_view setAutoresizingMask:NSViewWidthSizable];
  [s.toolbar_view addSubview:s.separator_view];

  s.progress_view = [[NSView alloc]
      initWithFrame:NSMakeRect(0, 0, 0, kProgressHeight)];
  [s.progress_view setWantsLayer:YES];
  s.progress_view.layer.backgroundColor = AegisAccent().CGColor;
  s.progress_view.layer.cornerRadius = 1.0;
  [s.progress_view setAutoresizingMask:NSViewMaxXMargin];
  s.progress_view.hidden = YES;
  [s.toolbar_view addSubview:s.progress_view positioned:NSWindowAbove
                                             relativeTo:s.separator_view];

  const CGFloat tab_strip_bottom = 44.0;
  const CGFloat tab_strip_band = 34.0;
  const CGFloat tab_item_y = tab_strip_bottom + (tab_strip_band - kTabHeight) / 2.0;

  s.tab_bg = [[NSView alloc]
      initWithFrame:NSMakeRect(kTabLeftInset, tab_item_y, kTabInitialWidth, kTabHeight)];
  [s.tab_bg setWantsLayer:YES];
  s.tab_bg.layer.cornerRadius = kTabRadius;
  s.tab_bg.layer.backgroundColor = [NSColor whiteColor].CGColor;
  s.tab_bg.layer.opaque = YES;
  s.tab_bg.layer.borderWidth = 0.5;
  s.tab_bg.layer.borderColor = AegisActiveTabBorder().CGColor;
  s.tab_bg.layer.shadowColor = [NSColor blackColor].CGColor;
  s.tab_bg.layer.shadowOffset = CGSizeMake(0, -1.0);
  s.tab_bg.layer.shadowRadius = 3.0;
  s.tab_bg.layer.shadowOpacity = 0.08;
  s.tab_bg.layer.masksToBounds = NO;
  [s.toolbar_view addSubview:s.tab_bg];

  s.tab_title_label = [NSTextField labelWithString:@"Aegis"];
  [s.tab_title_label setFrame:NSMakeRect(kTabHPad, 4.0,
                                         kTabInitialWidth - kTabHPad * 2.0,
                                         kTabHeight - 8.0)];
  s.tab_title_label.font = [NSFont systemFontOfSize:12.5 weight:NSFontWeightMedium];
  s.tab_title_label.textColor = AegisPrimaryText();
  s.tab_title_label.lineBreakMode = NSLineBreakByTruncatingTail;
  [s.tab_title_label setAutoresizingMask:NSViewWidthSizable];
  [s.tab_bg addSubview:s.tab_title_label];

  s.new_tab_button = [[AegisNavButton alloc] initWithSymbolName:@"plus"];
  const CGFloat ntb_x = kTabLeftInset + kTabInitialWidth + 6.0;
  const CGFloat ntb_size = 22.0;
  const CGFloat ntb_y = tab_item_y + (kTabHeight - ntb_size) / 2.0;
  [s.new_tab_button setFrame:NSMakeRect(ntb_x, ntb_y, ntb_size, ntb_size)];
  s.new_tab_button.layer.cornerRadius = ntb_size / 2.0;
  {
    NSImageSymbolConfiguration* cfg = [NSImageSymbolConfiguration
        configurationWithPointSize:10.0 weight:NSFontWeightMedium];
    NSImage* img = [[NSImage imageWithSystemSymbolName:@"plus"
                                 accessibilityDescription:@"New tab"]
        imageWithSymbolConfiguration:cfg];
    [s.new_tab_button setImage:img];
  }
  [s.new_tab_button setContentTintColor:AegisSecondaryText()];
  [s.new_tab_button setEnabled:NO];
  s.new_tab_button.alphaValue = 0.5;
  [s.toolbar_view addSubview:s.new_tab_button];

  const CGFloat nav_center_y = 22.0;
  const CGFloat nav_btn_y = nav_center_y - kNavBtnSize / 2.0;

  s.back_button = [[AegisNavButton alloc] initWithSymbolName:@"chevron.left"];
  CGFloat bx = kNavLeftInset;
  [s.back_button setFrame:NSMakeRect(bx, nav_btn_y, kNavBtnSize, kNavBtnSize)];
  [s.toolbar_view addSubview:s.back_button];

  s.forward_button = [[AegisNavButton alloc] initWithSymbolName:@"chevron.right"];
  bx += kNavBtnSize + kNavBtnGap;
  [s.forward_button setFrame:NSMakeRect(bx, nav_btn_y, kNavBtnSize, kNavBtnSize)];
  [s.toolbar_view addSubview:s.forward_button];

  s.reload_button = [[AegisNavButton alloc] initWithSymbolName:@"arrow.clockwise"];
  bx += kNavBtnSize + kNavBtnGap;
  [s.reload_button setFrame:NSMakeRect(bx, nav_btn_y, kNavBtnSize, kNavBtnSize)];
  [s.toolbar_view addSubview:s.reload_button];

  const CGFloat addr_x = bx + kNavBtnSize + kAddrGapFromNav;
  const CGFloat addr_w = w - addr_x - kAddrRightInset;
  const CGFloat addr_y = nav_center_y - kAddrHeight / 2.0;

  s.address_container = [[AegisAddressContainer alloc]
      initWithFrame:NSMakeRect(addr_x, addr_y, addr_w, kAddrHeight)];
  [s.address_container setAutoresizingMask:NSViewWidthSizable];
  [s.toolbar_view addSubview:s.address_container];

  {
    NSImageSymbolConfiguration* cfg = [NSImageSymbolConfiguration
        configurationWithPointSize:11.0 weight:NSFontWeightRegular];
    NSImage* lock_img = [[NSImage imageWithSystemSymbolName:@"lock.fill"
                                      accessibilityDescription:@"Secure"]
        imageWithSymbolConfiguration:cfg];

    s.lock_icon = [[NSImageView alloc]
        initWithFrame:NSMakeRect(kAddrHPad, (kAddrHeight - 14.0) / 2.0, 14.0, 14.0)];
    [s.lock_icon setImage:lock_img];
    [s.lock_icon setContentTintColor:AegisLockIcon()];
    [s.lock_icon setAutoresizingMask:NSViewMaxXMargin];
    [s.address_container addSubview:s.lock_icon];
  }

  const CGFloat field_x = kAddrHPad + 14.0 + 6.0;
  const CGFloat field_w = addr_w - field_x - kAddrHPad;
  const CGFloat field_h = 20.0;
  const CGFloat field_y = (kAddrHeight - field_h) / 2.0;
  s.address_field = [[AegisAddressField alloc]
      initWithFrame:NSMakeRect(field_x, field_y, field_w, field_h)];
  s.address_field.addressContainer = s.address_container;
  [s.address_field setAutoresizingMask:NSViewWidthSizable];
  [s.address_field setBezeled:NO];
  [s.address_field setBordered:NO];
  [s.address_field setFocusRingType:NSFocusRingTypeNone];
  [s.address_field setDrawsBackground:NO];
  [s.address_field setBackgroundColor:[NSColor clearColor]];
  [s.address_field setTextColor:AegisPrimaryText()];
  [s.address_field setFont:[NSFont systemFontOfSize:13.0 weight:NSFontWeightRegular]];
  [s.address_field setPlaceholderString:@"Search or enter address"];

  {
    NSAttributedString* placeholder = [[NSAttributedString alloc]
        initWithString:@"Search or enter address"
            attributes:@{
              NSForegroundColorAttributeName : AegisPlaceholderText(),
              NSFontAttributeName : [NSFont systemFontOfSize:13.0 weight:NSFontWeightRegular]
            }];
    [s.address_field setPlaceholderAttributedString:placeholder];
  }

  [s.address_field setLineBreakMode:NSLineBreakByTruncatingTail];
  [s.address_field setUsesSingleLineMode:YES];
  [s.address_container addSubview:s.address_field];

  s.web_container_view = [[NSView alloc]
      initWithFrame:NSMakeRect(0, 0, w, h - kToolbarHeight)];
  [s.web_container_view setAutoresizingMask:NSViewWidthSizable | NSViewHeightSizable];
  [s.web_container_view setWantsLayer:YES];
  [s.root_view addSubview:s.web_container_view];

  s.page_transition_view = [[NSView alloc]
      initWithFrame:[s.web_container_view bounds]];
  [s.page_transition_view setAutoresizingMask:NSViewWidthSizable | NSViewHeightSizable];
  [s.page_transition_view setWantsLayer:YES];
  s.page_transition_view.layer.backgroundColor = AegisPageTransitionFill().CGColor;
  s.page_transition_view.hidden = YES;
  s.page_transition_view.alphaValue = 0.0;
  [s.web_container_view addSubview:s.page_transition_view];

  s.controller = [[AegisBrowserWindowController alloc] initWithStatePointer:&s];
  [s.window setDelegate:s.controller];

  [s.back_button setTarget:s.controller];
  [s.back_button setAction:@selector(navigateBack:)];
  [s.forward_button setTarget:s.controller];
  [s.forward_button setAction:@selector(navigateForward:)];
  [s.reload_button setTarget:s.controller];
  [s.reload_button setAction:@selector(reloadPage:)];
  [s.address_field setTarget:s.controller];
  [s.address_field setAction:@selector(submitLocation:)];
  [s.address_field setDelegate:s.controller];

  [s.controller updateAddress:[NSString stringWithUTF8String:title.c_str()]];
  [s.controller updateNavigationButtonsWithCanGoBack:NO
                                        canGoForward:NO
                                           isLoading:NO];
}

}  // namespace

@implementation AegisBrowserWindowController {
  void* _state_ptr;
}

- (instancetype)initWithStatePointer:(void*)state {
  self = [super init];
  if (self) {
    _state_ptr = state;
  }
  return self;
}

- (BrowserHostState*)state {
  return static_cast<BrowserHostState*>(_state_ptr);
}

- (void)navigateBack:(id)sender {
  (void)sender;
  auto* state = [self state];
  if (state->browser && state->browser->CanGoBack()) {
    state->browser->GoBack();
  }
}

- (void)navigateForward:(id)sender {
  (void)sender;
  auto* state = [self state];
  if (state->browser && state->browser->CanGoForward()) {
    state->browser->GoForward();
  }
}

- (void)reloadPage:(id)sender {
  (void)sender;
  auto* state = [self state];
  if (state->browser) {
    if (state->is_loading) {
      state->browser->StopLoad();
    } else {
      state->browser->Reload();
    }
  }
}

- (void)submitLocation:(id)sender {
  (void)sender;
  auto* state = [self state];
  if (!state->browser) {
    return;
  }
  NSString* value = [[state->address_field stringValue]
      stringByTrimmingCharactersInSet:[NSCharacterSet whitespaceAndNewlineCharacterSet]];
  if (value.length == 0) {
    return;
  }
  if ([value rangeOfString:@"://"].location == NSNotFound) {
    value = [@"https://" stringByAppendingString:value];
  }
  state->browser->GetMainFrame()->LoadURL(std::string([value UTF8String]));
  [self unfocusAddressBar];
}

- (void)updateAddress:(NSString*)address {
  auto* state = [self state];
  if (address == nil || state->address_field == nil) {
    return;
  }
  if (![[state->address_field currentEditor] isKindOfClass:[NSTextView class]]) {
    [state->address_field setStringValue:address];
  }

  if (state->lock_icon) {
    BOOL is_secure = [address hasPrefix:@"https://"];
    BOOL is_empty = (address.length == 0);
    state->lock_icon.hidden = (!is_secure && !is_empty) || is_empty;
  }
}

- (void)updateTitle:(NSString*)title {
  auto* state = [self state];
  if (title == nil) return;

  [state->window setTitle:title];
  if (state->tab_title_label) {
    [state->tab_title_label setStringValue:title];
    NSDictionary* attrs = @{
      NSFontAttributeName : state->tab_title_label.font
    };
    CGFloat text_w = [title sizeWithAttributes:attrs].width;
    CGFloat tab_w = text_w + kTabHPad * 2.0 + 4.0;
    tab_w = fmax(80.0, fmin(tab_w, 240.0));
    NSRect tf = state->tab_bg.frame;
    tf.size.width = tab_w;
    [state->tab_bg setFrame:tf];

    if (state->new_tab_button) {
      NSRect ntf = state->new_tab_button.frame;
      ntf.origin.x = tf.origin.x + tf.size.width + 6.0;
      [state->new_tab_button setFrame:ntf];
    }
  }
}

- (void)updateNavigationButtonsWithCanGoBack:(BOOL)canGoBack
                                 canGoForward:(BOOL)canGoForward
                                    isLoading:(BOOL)isLoading {
  auto* state = [self state];
  [state->back_button setEnabled:canGoBack];
  [state->forward_button setEnabled:canGoForward];

  if (isLoading) {
    [state->reload_button setSymbolName:@"xmark"];
  } else {
    [state->reload_button setSymbolName:@"arrow.clockwise"];
  }

  if (state->progress_view) {
    [state->progress_view.layer removeAllAnimations];
    if (isLoading) {
      CGFloat toolbar_w = state->toolbar_view.frame.size.width;
      state->progress_view.hidden = NO;
      [state->progress_view setAlphaValue:1.0];
      [state->progress_view setFrame:NSMakeRect(0, kSeparatorHeight, 0, kProgressHeight)];

      [NSAnimationContext runAnimationGroup:^(NSAnimationContext* ctx) {
        ctx.duration = 8.0;
        ctx.timingFunction = [CAMediaTimingFunction
            functionWithName:kCAMediaTimingFunctionEaseOut];
        [[state->progress_view animator]
            setFrame:NSMakeRect(0, kSeparatorHeight,
                                toolbar_w * 0.75, kProgressHeight)];
      } completionHandler:nil];
    } else {
      CGFloat toolbar_w = state->toolbar_view.frame.size.width;
      [NSAnimationContext runAnimationGroup:^(NSAnimationContext* ctx) {
        ctx.duration = 0.2;
        [[state->progress_view animator]
            setFrame:NSMakeRect(0, kSeparatorHeight,
                                toolbar_w, kProgressHeight)];
      } completionHandler:^{
        [NSAnimationContext runAnimationGroup:^(NSAnimationContext* ctx) {
          ctx.duration = 0.3;
          [[state->progress_view animator] setAlphaValue:0.0];
        } completionHandler:^{
          state->progress_view.hidden = YES;
          [state->progress_view setAlphaValue:1.0];
        }];
      }];
    }
  }

  if (state->page_transition_view) {
    [state->page_transition_view.layer removeAllAnimations];
    state->transition_generation += 1;
    const std::uint64_t generation = state->transition_generation;
    if (isLoading) {
      state->loading_started_at = CFAbsoluteTimeGetCurrent();
      dispatch_after(
          dispatch_time(DISPATCH_TIME_NOW,
                        static_cast<int64_t>(kPageTransitionDelay * NSEC_PER_SEC)),
          dispatch_get_main_queue(), ^{
            if (state->transition_generation != generation || !state->is_loading ||
                state->page_transition_view == nil) {
              return;
            }
            state->page_transition_view.hidden = NO;
            [state->page_transition_view setAlphaValue:0.0];
            [NSAnimationContext runAnimationGroup:^(NSAnimationContext* ctx) {
              ctx.duration = kPageTransitionFadeInDuration;
              ctx.timingFunction = [CAMediaTimingFunction
                  functionWithName:kCAMediaTimingFunctionEaseOut];
              [[state->page_transition_view animator] setAlphaValue:kPageTransitionMaxAlpha];
            } completionHandler:^{
              if (state->transition_generation == generation) {
                state->transition_visible_since = CFAbsoluteTimeGetCurrent();
              }
            }];
          });
    } else {
      CFTimeInterval delay = 0;
      if (state->transition_visible_since > 0) {
        const CFTimeInterval visible_for =
            CFAbsoluteTimeGetCurrent() - state->transition_visible_since;
        delay = fmax(0.0, kPageTransitionMinVisibleDuration - visible_for);
      }
      dispatch_after(
          dispatch_time(DISPATCH_TIME_NOW,
                        static_cast<int64_t>(delay * NSEC_PER_SEC)),
          dispatch_get_main_queue(), ^{
            if (state->transition_generation != generation || state->is_loading ||
                state->page_transition_view == nil) {
              return;
            }
            if (!state->page_transition_view.hidden ||
                state->page_transition_view.alphaValue > 0.0) {
              [NSAnimationContext runAnimationGroup:^(NSAnimationContext* ctx) {
                ctx.duration = kPageTransitionFadeOutDuration;
                ctx.timingFunction = [CAMediaTimingFunction
                    functionWithName:kCAMediaTimingFunctionEaseInEaseOut];
                [[state->page_transition_view animator] setAlphaValue:0.0];
              } completionHandler:^{
                if (state->transition_generation != generation ||
                    state->page_transition_view == nil) {
                  return;
                }
                state->page_transition_view.hidden = YES;
                state->transition_visible_since = 0;
              }];
            } else {
              state->page_transition_view.hidden = YES;
              state->transition_visible_since = 0;
            }
          });
      if (state->page_transition_view.hidden && state->page_transition_view.alphaValue <= 0.0) {
        state->transition_visible_since = 0;
      }
    }
  }
}

- (void)unfocusAddressBar {
  auto* state = [self state];
  [state->window makeFirstResponder:nil];
  if (state->address_container) {
    [state->address_container setFocused:NO];
  }
}

- (void)controlTextDidEndEditing:(NSNotification*)notification {
  (void)notification;
  auto* state = [self state];
  if (state->address_container) {
    [state->address_container setFocused:NO];
  }
}

- (BOOL)windowShouldClose:(NSWindow*)window {
  (void)window;
  BrowserHostState& state = HostState();
  if (state.closing_from_browser) {
    return YES;
  }
  if (state.close_requested_by_user) {
    return NO;
  }
  state.close_requested_by_user = true;
  if (state.window) {
    [state.window orderOut:nil];
  }
  if (state.browser) {
    state.browser->GetHost()->CloseBrowser(true);
    return NO;
  }
  [NSApp terminate:nil];
  return YES;
}

@end

void AegisInitializeBrowserHostApplication() {
  InitializeHostApplication();
}

CefWindowHandle AegisCreateBrowserHostView(const std::string& title,
                                           int width,
                                           int height) {
  EnsureHostWindow(title, width, height);
  return CAST_NSVIEW_TO_CEF_WINDOW_HANDLE(HostState().web_container_view);
}

void AegisShowBrowserHostWindow() {
  auto& state = HostState();
  if (state.window == nil) {
    return;
  }
  [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
  [NSApp activateIgnoringOtherApps:YES];
  [state.window makeKeyAndOrderFront:nil];
}

void AegisSetBrowserHostTitle(const std::string& title) {
  auto& state = HostState();
  if (state.window == nil) {
    return;
  }
  state.current_title = title;
  NSString* value = [NSString stringWithUTF8String:title.c_str()];
  [state.controller updateTitle:value];
}

void AegisSetBrowserHostAddress(const std::string& url) {
  auto& state = HostState();
  state.current_url = url;
  if (state.controller == nil) {
    return;
  }
  NSString* value = [NSString stringWithUTF8String:url.c_str()];
  [state.controller updateAddress:value];
}

void AegisSetBrowserHostNavigationState(bool can_go_back,
                                        bool can_go_forward,
                                        bool is_loading) {
  auto& state = HostState();
  state.can_go_back = can_go_back;
  state.can_go_forward = can_go_forward;
  state.is_loading = is_loading;
  if (state.controller == nil) {
    return;
  }
  [state.controller updateNavigationButtonsWithCanGoBack:can_go_back
                                            canGoForward:can_go_forward
                                               isLoading:is_loading];
}

void AegisAttachBrowserToHostWindow(CefRefPtr<CefBrowser> browser) {
  auto& state = HostState();
  if (state.window == nil || state.web_container_view == nil || !browser) {
    return;
  }

  state.browser = browser;
  NSView* browser_view =
      CAST_CEF_WINDOW_HANDLE_TO_NSVIEW(browser->GetHost()->GetWindowHandle());
  if (browser_view == nil) {
    return;
  }

  [browser_view setFrame:[state.web_container_view bounds]];
  [browser_view setAutoresizingMask:NSViewWidthSizable | NSViewHeightSizable];
  if ([browser_view superview] != state.web_container_view) {
    [browser_view removeFromSuperview];
    [state.web_container_view addSubview:browser_view
                              positioned:NSWindowBelow
                              relativeTo:state.page_transition_view];
  }
}

void AegisCloseBrowserHostWindow() {
  auto& state = HostState();
  state.browser = nullptr;
  if (state.window == nil) {
    return;
  }
  state.closing_from_browser = true;
  [state.window orderOut:nil];
  [state.window close];
  state.closing_from_browser = false;
  state.window = nil;
  state.root_view = nil;
  state.toolbar_view = nil;
  state.separator_view = nil;
  state.tab_bg = nil;
  state.tab_title_label = nil;
  state.new_tab_button = nil;
  state.back_button = nil;
  state.forward_button = nil;
  state.reload_button = nil;
  state.address_container = nil;
  state.lock_icon = nil;
  state.address_field = nil;
  state.progress_view = nil;
  state.web_container_view = nil;
  state.page_transition_view = nil;
  state.controller = nil;
  state.close_requested_by_user = false;
  state.loading_started_at = 0;
  state.transition_visible_since = 0;
  state.transition_generation = 0;
  [NSApp terminate:nil];
}

void AegisPumpBrowserHostWindow() {
  if (NSApp == nil) {
    return;
  }

  for (;;) {
    NSEvent* event = [NSApp nextEventMatchingMask:NSEventMaskAny
                                        untilDate:[NSDate distantPast]
                                           inMode:NSDefaultRunLoopMode
                                          dequeue:YES];
    if (event == nil) {
      break;
    }
    [NSApp sendEvent:event];
  }

  [NSApp updateWindows];
  (void)AegisRunScheduledCefMessagePumpWorkIfDue();
}

bool AegisBrowserHostWindowCloseRequested() {
  auto& state = HostState();
  return state.close_requested_by_user;
}
