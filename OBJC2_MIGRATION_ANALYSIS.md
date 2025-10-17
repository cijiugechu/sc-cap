# objc2 Migration Analysis

## Overview

This document analyzes the migration path from current macOS-specific dependencies to the modern `objc2` ecosystem for the `sc-cap` screen capture library.

## Current Dependencies Analysis

### Existing macOS Dependencies

```toml
[target.'cfg(target_os = "macos")'.dependencies]
core-graphics = "0.24.0"
cocoa = "0.25.0"
objc = "0.2.7"
cidre = { version = "0.10.1", default-features = false, features = [
    "async",
    "av",
    "sc",
    "dispatch",
    "macos_13_0",
] }
```

### Current Usage Patterns

The current dependencies are used extensively in `src/capturer/engine/mac/`:

- **core-graphics**: Used for `CGDisplay` and `CGDisplayMode` in display operations
- **objc**: Provides Objective-C runtime interactions
- **cidre**: Heavy usage for ScreenCaptureKit functionality including:
  - Screen capture (`sc` module)
  - CoreGraphics (`cg` module)
  - CoreMedia (`cm` module) 
  - CoreVideo (`cv` module)
  - Dispatch queues (`dispatch` module)
  - NSObject/ARC memory management (`arc`, `objc` modules)

## Recommended objc2 Replacements

### Dependency Mapping

| Current Crate | objc2 Replacement | Purpose |
|---------------|-------------------|---------|
| `core-graphics` | `objc2-core-graphics` + `objc2-core-foundation` | CoreGraphics framework |
| `cocoa` | `objc2-app-kit` + `objc2-foundation` | AppKit framework |
| `objc` | `objc2` | Objective-C runtime |
| `cidre` | Multiple objc2 crates | Comprehensive framework bindings |

### Proposed New Dependencies

```toml
[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "0.6"
objc2-core-graphics = "0.3"
objc2-core-foundation = "0.3"
objc2-core-media = "0.3"
objc2-core-video = "0.3"
objc2-foundation = "0.3"
objc2-screen-capture-kit = "0.3"
dispatch2 = "0.3"
```

### Crate Functionality Overview

#### Core Framework Bindings
- **`objc2-core-graphics`**: Provides `CGContext`, `CGImage`, `CGColorSpace`, and drawing operations
- **`objc2-core-foundation`**: Core Foundation types and utilities
- **`objc2-foundation`**: Foundation classes like `NSObject`, `NSString`, `NSArray`

#### Media and Capture
- **`objc2-core-media`**: Core Media framework for media processing
- **`objc2-core-video`**: Core Video for pixel buffer and video processing
- **`objc2-screen-capture-kit`**: ScreenCaptureKit framework for screen recording

#### System Integration
- **`objc2-app-kit`**: AppKit framework for UI components (if needed)
- **`dispatch2`**: Grand Central Dispatch for concurrent operations
- **`objc2`**: Base Objective-C runtime and message sending

## Migration Considerations

### 1. API Changes
The objc2 ecosystem uses more idiomatic Rust patterns:
- Better error handling with `Result` types
- More ergonomic memory management
- Improved type safety

### 2. ScreenCaptureKit Migration
The most significant changes will be in ScreenCaptureKit usage:

**Current (cidre)**:
```rust
use cidre::{
    sc::{self, StreamDelegate, StreamOutput, StreamOutputImpl},
    cg, cm, cv, arc, ns, objc,
};
```

**New (objc2)**:
```rust
use objc2_screen_capture_kit::*;
use objc2_core_graphics::*;
use objc2_core_media::*;
use objc2_core_video::*;
use objc2_foundation::*;
```

### 3. Memory Management
objc2 handles ARC (Automatic Reference Counting) differently:
- More explicit ownership patterns
- Better integration with Rust's ownership system
- Reduced manual memory management

### 4. Error Handling
objc2 provides better Rust-native error handling:
- `Result<T, E>` patterns instead of Objective-C exceptions
- More descriptive error types
- Better error propagation

## Migration Strategy

### Phase 1: Dependency Updates
1. Update `Cargo.toml` with new objc2 dependencies
2. Remove old dependencies
3. Resolve any feature flag requirements

### Phase 2: CoreGraphics Migration
1. Replace `core-graphics` usage with `objc2-core-graphics`
2. Update `ext.rs` display functionality
3. Test basic graphics operations

### Phase 3: ScreenCaptureKit Migration
1. Replace `cidre` ScreenCaptureKit usage with `objc2-screen-capture-kit`
2. Update stream delegate and output implementations
3. Migrate CoreMedia and CoreVideo usage
4. Update memory management patterns

### Phase 4: Testing and Validation
1. Test all screen capture functionality
2. Verify performance characteristics
3. Ensure compatibility with existing APIs

## Benefits of Migration

1. **Modern Rust Patterns**: More idiomatic and safer code
2. **Better Documentation**: Comprehensive docs.rs documentation
3. **Active Maintenance**: objc2 ecosystem is actively maintained
4. **Type Safety**: Improved compile-time guarantees
5. **Memory Safety**: Better integration with Rust's ownership model
6. **Future Compatibility**: Forward-compatible with macOS updates

## Potential Challenges

1. **API Differences**: Significant changes in some APIs
2. **Learning Curve**: Team familiarity with objc2 patterns
3. **Testing Extensive**: Comprehensive testing required for screen capture functionality
4. **Migration Time**: Requires careful, incremental migration

## Conclusion

The migration to objc2 ecosystem provides significant long-term benefits for maintainability, safety, and modern Rust integration. While the migration requires careful planning and testing, the result will be a more robust and idiomatic Rust codebase that's better positioned for future development.

The ScreenCaptureKit functionality will require the most attention due to its extensive current usage, but the objc2-screen-capture-kit crate provides equivalent functionality with modern Rust patterns.