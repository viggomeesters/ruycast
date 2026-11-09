// Complete owned SDK provider. AppKit may ask for the general board object while
// constructing an NSTextView; replace the object itself before main, so even
// framework-internal reads see a private named board and never personal data.
#include "snippet_effects_shim.m"
static NSPasteboard *ownedFrameworkBoard;
static id ownFrameworkBoard(id self,SEL selector){
    (void)self;(void)selector;
    if(!ownedFrameworkBoard)ownedFrameworkBoard=[NSPasteboard pasteboardWithUniqueName];
    return ownedFrameworkBoard;
}
__attribute__((constructor)) static void installClipboardHistory(void){
    @autoreleasepool {
        if(!fixtureHome)return;
        Method method=class_getClassMethod([NSPasteboard class],@selector(generalPasteboard));
        method_setImplementation(method,(IMP)ownFrameworkBoard);
    }
}
__attribute__((visibility("default"))) NSPasteboard *ruycast_clipboard_history_fixture_board(void){
    static NSPasteboard *board;
    if(!fixtureHome)return nil;
    if(!board)board=[NSPasteboard pasteboardWithName:[@"ruycast-clipboard-history-fixture-" stringByAppendingString:NSUUID.UUID.UUIDString]];
    return board;
}
