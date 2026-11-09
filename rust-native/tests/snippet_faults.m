// Test-only per-process Objective-C SDK faults. Only the two own fixture
// executables load this library; the general pasteboard accessor aborts.
#import <AppKit/AppKit.h>
#import <objc/runtime.h>
#include <stdlib.h>
#include <string.h>

static const char *fault;
static BOOL (*originalSet)(id,SEL,NSString *,NSString *);
static BOOL (*originalWrite)(id,SEL,NSArray *);
static id forbiddenGeneral(id self,SEL selector) { abort(); }
static BOOL setString(id self,SEL selector,NSString *text,NSString *type) {
    if ((!strcmp(fault,"string") && [type isEqualToString:NSPasteboardTypeString]) ||
        (!strcmp(fault,"html") && [type isEqualToString:NSPasteboardTypeHTML])) return NO;
    return originalSet(self,selector,text,type);
}
static BOOL writeObjects(id self,SEL selector,NSArray *objects) {
    if ([[self name] isEqualToString:NSPasteboardNameGeneral]) abort();
    if (!strcmp(fault,"write")) return NO;
    return originalWrite(self,selector,objects);
}
__attribute__((constructor)) static void install(void) {
    @autoreleasepool {
        const char *guard=getenv("RUYCAST_SNIPPET_FIXTURE");
        fault=getenv("RUYCAST_SNIPPET_FAULT");
        if (!guard || strcmp(guard,"own-named-board") || !fault) abort();
        Method general=class_getClassMethod([NSPasteboard class],@selector(generalPasteboard));
        Method set=class_getInstanceMethod([NSPasteboardItem class],@selector(setString:forType:));
        Method write=class_getInstanceMethod([NSPasteboard class],@selector(writeObjects:));
        if (!general || !set || !write) abort();
        method_setImplementation(general,(IMP)forbiddenGeneral);
        originalSet=(void *)method_setImplementation(set,(IMP)setString);
        originalWrite=(void *)method_setImplementation(write,(IMP)writeObjects);
    }
}
