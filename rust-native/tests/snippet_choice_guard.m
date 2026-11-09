// AppKit itself may validate paste actions during control construction. Replace
// every general-board lookup with one owned empty named board; never call the
// system board implementation. No user contents or real global input.
#import <AppKit/AppKit.h>
#import <objc/runtime.h>
#include <assert.h>
#include <string.h>
static NSPasteboard *ownBoard;
static bool installed;
@interface RuycastChoiceMonitor : NSObject
@property(nonatomic,copy) void (^handler)(NSEvent *);
@end
@implementation RuycastChoiceMonitor @end
static NSMutableArray<RuycastChoiceMonitor *> *monitors;
static NSMutableArray *localMonitors;
static id (*nativeLocalAdd)(id,SEL,NSEventMask,NSEvent *(^)(NSEvent *));
static void (*nativeRemove)(id,SEL,id);
static id ownLocalMonitor(id object,SEL selector,NSEventMask mask,NSEvent *(^handler)(NSEvent *)){
    // AppKit's process-local editing observers keep their actual semantics.
    // Only tokens from this explicit local registration may reach native remove.
    id token=nativeLocalAdd(object,selector,mask,handler);
    if(token)[localMonitors addObject:token];return token;
}
static uint64_t monitorCreates,monitorRemoves,monitorDeliveries;
static void holdOwnRemoval(void){
    NSString *root=[NSProcessInfo processInfo].environment[@"RUYCAST_KEYWORD_FIXTURE_HOME"];
    if(!root)return;
    NSString *hold=[root stringByAppendingPathComponent:@"hold-choice-remove"];
    if(![[NSFileManager defaultManager] fileExistsAtPath:hold])return;
    NSString *stamp=[NSString stringWithContentsOfFile:[root stringByAppendingPathComponent:@".ruycast-keyboard-fixture"] encoding:NSUTF8StringEncoding error:NULL];assert([stamp isEqualToString:@"own-keyboard-fixture-v1"]);
    [@"entered" writeToFile:[root stringByAppendingPathComponent:@"choice-remove-entered"] atomically:YES encoding:NSUTF8StringEncoding error:NULL];
    NSDate *deadline=[NSDate dateWithTimeIntervalSinceNow:5];
    while(![[NSFileManager defaultManager] fileExistsAtPath:[root stringByAppendingPathComponent:@"choice-remove-release"]]&&[deadline timeIntervalSinceNow]>0){
        [[NSRunLoop currentRunLoop] runMode:NSDefaultRunLoopMode beforeDate:[NSDate dateWithTimeIntervalSinceNow:.002]];
        [NSThread sleepForTimeInterval:.001];
    }
}
static id ownMonitor(id object,SEL selector,NSEventMask mask,void (^handler)(NSEvent *)){
    (void)object;(void)selector;
    assert(installed&&monitors&&handler&&mask==(NSEventMaskLeftMouseDown|NSEventMaskRightMouseDown));
    RuycastChoiceMonitor *monitor=[RuycastChoiceMonitor new];monitor.handler=handler;
    [monitors addObject:monitor];monitorCreates++;return monitor;
}
static void ownRemoveMonitor(id object,SEL selector,id value){
    (void)object;(void)selector;
    // AppKit startup also removes its absent monitor. This owns no hook.
    if(!value)return;
    if([localMonitors indexOfObjectIdenticalTo:value]!=NSNotFound){
        nativeRemove(object,selector,value);[localMonitors removeObjectIdenticalTo:value];return;
    }
    // This AppKit runtime also creates its own process-local observer through
    // a private internal path. Its observed local class may retire normally;
    // no unknown/global observer is ever forwarded to the native remover.
    if(strcmp(class_getName(object_getClass(value)),"_NSLocalEventObserver")==0){nativeRemove(object,selector,value);return;}
    assert([value isKindOfClass:[RuycastChoiceMonitor class]]);
    RuycastChoiceMonitor *monitor=value;assert([monitors indexOfObjectIdenticalTo:monitor]!=NSNotFound);
    holdOwnRemoval();
    monitor.handler=nil;[monitors removeObjectIdenticalTo:monitor];monitorRemoves++;
}
__attribute__((visibility("default"))) bool ruycast_choice_fixture_monitor_guard(void){return installed&&monitors!=nil;}
__attribute__((visibility("default"))) uint64_t ruycast_choice_fixture_click(void){
    assert(installed);uint64_t delivered=0;
    NSEvent *event=[NSEvent mouseEventWithType:NSEventTypeLeftMouseDown location:NSMakePoint(0,0) modifierFlags:0 timestamp:0 windowNumber:0 context:nil eventNumber:0 clickCount:1 pressure:0];assert(event);
    for(RuycastChoiceMonitor *monitor in [monitors copy]){
        void (^handler)(NSEvent *)=monitor.handler;
        if(handler){handler(event);delivered++;monitorDeliveries++;}
    }
    return delivered;
}
__attribute__((visibility("default"))) void ruycast_choice_fixture_schedule_click(void){
    // Deliver outside the control source's own receiver frame. The held remover
    // pumps a nested runloop, allowing actual stop/retirement to be observed.
    [NSTimer scheduledTimerWithTimeInterval:.01 repeats:NO block:^(__unused NSTimer *timer){ruycast_choice_fixture_click();}];
}
__attribute__((visibility("default"))) void ruycast_choice_fixture_monitor_counts(uint64_t *created,uint64_t *removed,uint64_t *delivered,uint64_t *active){
    assert(created&&removed&&delivered&&active);*created=monitorCreates;*removed=monitorRemoves;*delivered=monitorDeliveries;*active=[monitors count];
}
static id ownGeneral(id object,SEL selector){(void)object;(void)selector;assert(installed&&ownBoard);return ownBoard;}
__attribute__((constructor)) static void setup(void){
    @autoreleasepool {
        Method method=class_getClassMethod([NSPasteboard class],@selector(generalPasteboard));assert(method);
        installed=method_setImplementation(method,(IMP)ownGeneral)!=NULL;assert(installed);
        ownBoard=[NSPasteboard pasteboardWithUniqueName];assert(ownBoard);[ownBoard clearContents];
        monitors=[NSMutableArray new];localMonitors=[NSMutableArray new];
        Method add=class_getClassMethod([NSEvent class],@selector(addGlobalMonitorForEventsMatchingMask:handler:));
        Method remove=class_getClassMethod([NSEvent class],@selector(removeMonitor:));assert(add&&remove);
        Method local=class_getClassMethod([NSEvent class],@selector(addLocalMonitorForEventsMatchingMask:handler:));assert(local);
        nativeLocalAdd=(void *)method_setImplementation(local,(IMP)ownLocalMonitor);assert(nativeLocalAdd);
        assert(method_setImplementation(add,(IMP)ownMonitor));nativeRemove=(void *)method_setImplementation(remove,(IMP)ownRemoveMonitor);assert(nativeRemove);
    }
}
__attribute__((visibility("default"))) bool ruycast_choice_fixture_guard(void){return installed&&ownBoard!=nil;}
__attribute__((destructor)) static void cleanup(void){@autoreleasepool{assert([monitors count]==0);[ownBoard clearContents];[ownBoard releaseGlobally];}}
