// Fault provider for one explicitly attached, unique named board. No general
// pasteboard access. Return genuine NSString values, including ill-formed UTF-16.
#import <AppKit/AppKit.h>
#import <objc/runtime.h>
#include <assert.h>
static NSPasteboard *ownedBoard;
static NSArray *ownedItems;
static IMP originalItems;
static Method itemsMethod;
static IMP originalString;
static Method stringMethod;
static int faultMode;
static bool generalGuardInstalled;
static id forbiddenGeneral(id object, SEL selector) {
    (void)object;(void)selector;
    abort();
}
__attribute__((constructor)) static void guardGeneral(void) {
    Method method=class_getClassMethod([NSPasteboard class],@selector(generalPasteboard));
    assert(method);
    generalGuardInstalled=method_setImplementation(method,(IMP)forbiddenGeneral)!=NULL;
    assert(generalGuardInstalled);
}
__attribute__((visibility("default"))) bool ruycast_owned_board_general_guard(void) {
    return generalGuardInstalled;
}
static NSString *units(const unichar *s, NSUInteger n) {
    return [[NSString alloc] initWithCharacters:s length:n];
}
@interface RuycastOwnedFaultItem : NSPasteboardItem
@end
@implementation RuycastOwnedFaultItem
- (NSArray<NSString *> *)types {
    if (faultMode == 3) { unichar u[] = {0xD800}; return @[units(u,1)]; }
    if (faultMode == 4) return @[NSPasteboardTypeString];
    if (faultMode == 6) return @[NSPasteboardTypeString,NSPasteboardTypeHTML,@"own.extra"];
    return @[NSPasteboardTypeHTML,NSPasteboardTypeString];
}
- (NSString *)stringForType:(NSPasteboardType)type {
    if ([type isEqualToString:NSPasteboardTypeString]) {
        if (faultMode == 10) { unichar u[] = {0xDC00}; return units(u,1); }
        if (faultMode == 11) { unichar u[] = {' ',0xD800,0xD800,0xDC00,0xDFFF,0,'\n'}; return units(u,7); }
        if (faultMode == 12 || faultMode == 13) {
            unichar u[4097];for (int i=0;i<4095;i++)u[i]='x';u[4095]=0xD83D;u[4096]=faultMode==12?0xDE00:'A';return units(u,4097);
        }
        if (faultMode == 14) {
            NSMutableString *large=[NSMutableString stringWithCapacity:1000005];
            [large appendString:@" \n"];for(int i=0;i<1000001;i++)[large appendString:@"x"];[large appendString:@"\n "];return [large copy];
        }
        if (faultMode == 15) return @" \n\u200b";
        if (faultMode == 16) return @"";
        if (faultMode == 17) return nil;
        if (faultMode == 1) { unichar u[] = {0xD800}; return units(u,1); }
        if (faultMode == 7) { unichar u[] = {'A',0,'C',0xD83D,0xDE00}; return units(u,5); }
        if (faultMode == 8 || faultMode == 9) {
            NSMutableString *prefix=[NSMutableString stringWithCapacity:5005];
            for (int i=0;i<5000;i++) [prefix appendString:@"x"];
            unichar u[] = {'A',0,faultMode==8?'C':'B',0xD83D,0xDE00};
            [prefix appendString:units(u,5)];
            return [prefix copy];
        }
        unichar u[] = {'A',0,'B',0xD83D,0xDE00}; return units(u,5);
    }
    if ([type isEqualToString:NSPasteboardTypeHTML]) {
        if (faultMode == 2) { unichar u[] = {0xDC00}; return units(u,1); }
        if (faultMode == 4) return nil;
        if (faultMode == 5) return @"<i>foreign</i>";
        return @"<b>A</b>";
    }
    return nil;
}
@end
static id fixtureItems(id object, SEL selector) {
    if (object == ownedBoard) return ownedItems;
    return ((id(*)(id,SEL))originalItems)(object,selector);
}
static id fixtureString(id object, SEL selector, id type) {
    if (object == ownedBoard) return [ownedItems[0] stringForType:type];
    return ((id(*)(id,SEL,id))originalString)(object,selector,type);
}
__attribute__((visibility("default"))) bool ruycast_owned_board_attach(NSPasteboard *board, int mode) {
    assert([NSThread isMainThread]);
    assert(!ownedBoard && board && ![board.name isEqualToString:NSPasteboardNameGeneral]);
    faultMode=mode;
    ownedBoard=board;
    ownedItems=@[[RuycastOwnedFaultItem new]];
    itemsMethod=class_getInstanceMethod(object_getClass(board),@selector(pasteboardItems));
    if (!itemsMethod) return false;
    originalItems=method_setImplementation(itemsMethod,(IMP)fixtureItems);
    stringMethod=class_getInstanceMethod(object_getClass(board),@selector(stringForType:));
    assert(stringMethod);
    originalString=method_setImplementation(stringMethod,(IMP)fixtureString);
    return originalItems != NULL && originalString != NULL;
}
__attribute__((visibility("default"))) void ruycast_owned_board_detach(void) {
    assert([NSThread isMainThread]);
    if (originalItems) method_setImplementation(itemsMethod,originalItems);
    if (originalString) method_setImplementation(stringMethod,originalString);
    originalString=NULL;
    originalItems=NULL;ownedItems=nil;ownedBoard=nil;
}
