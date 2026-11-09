// Own-process SDK replacement. Workspace selectors remain fully synthetic.
// Fixed ABI event entries return owned CF dictionaries, never HID events.
#include "workspace_shim.m"
#import <ApplicationServices/ApplicationServices.h>
static uint64_t snippetInstalled;
static IMP snippetOriginalSet;
static IMP snippetOriginalWrite;
static IMP snippetOriginalClear;
static NSPasteboard *lastSnippetBoard;
static BOOL flag(NSString *name){return [[NSFileManager defaultManager] fileExistsAtPath:[fixtureHome stringByAppendingPathComponent:name]];}
static void hold(NSString *phase){
    if(!flag([@"hold-" stringByAppendingString:phase]))return;
    [@"entered" writeToFile:[fixtureHome stringByAppendingPathComponent:[phase stringByAppendingString:@"-entered"]] atomically:YES encoding:NSUTF8StringEncoding error:NULL];
    while(!flag(@"snippet-release"))usleep(2000);
}
static void snippetRecord(NSString *phase,NSDictionary *fields){
    NSMutableDictionary *entry=[fields mutableCopy];
    struct timespec now;clock_gettime(CLOCK_MONOTONIC,&now);
    entry[@"atNs"]=@((uint64_t)now.tv_sec*1000000000+(uint64_t)now.tv_nsec);
    record(phase,entry);
}
static pid_t frontPID(void){
    NSString *value=[NSString stringWithContentsOfFile:[fixtureHome stringByAppendingPathComponent:@"frontmost-pid"] encoding:NSUTF8StringEncoding error:NULL];
    return value?(pid_t)value.intValue:4260;
}
static void setFrontPID(pid_t pid){[[NSString stringWithFormat:@"%d",pid] writeToFile:[fixtureHome stringByAppendingPathComponent:@"frontmost-pid"] atomically:YES encoding:NSUTF8StringEncoding error:NULL];}
@interface SnippetFixtureApp:NSObject
@property(nonatomic) pid_t pid;
@property(nonatomic) NSUInteger generation;
-(pid_t)processIdentifier;
-(BOOL)activateWithOptions:(NSApplicationActivationOptions)options;
-(NSString *)bundleIdentifier;
-(NSString *)localizedName;
@end
@implementation SnippetFixtureApp
-(pid_t)processIdentifier{return self.pid;}
-(BOOL)isTerminated{return flag(@"target-terminated");}
-(BOOL)isEqual:(id)other{return [other isKindOfClass:[SnippetFixtureApp class]] && self.pid==[other pid] && self.generation==[other generation];}
-(NSString *)bundleIdentifier{return @"dev.fixture.snippet";}
-(NSString *)localizedName{return @"Snippet fixture target";}
-(BOOL)activateWithOptions:(NSApplicationActivationOptions)options{
    (void)options;requireHide();snippetRecord(@"snippet-activate",@{@"pid":@(self.pid)});
    if(flag(@"activation-noop"))return NO;
    setFrontPID(self.pid);return YES;
}
@end
static id app(pid_t pid){SnippetFixtureApp *a=[SnippetFixtureApp new];a.pid=pid;a.generation=flag(@"target-new-generation")?1:0;return a;}
static id frontmost(id self,SEL selector){(void)self;(void)selector;return app(frontPID());}
static id lookup(id self,SEL selector,pid_t pid){
    (void)self;(void)selector;snippetRecord(@"snippet-lookup",@{@"pid":@(pid)});
    if(flag(@"missing-target"))return nil;
    setFrontPID(getpid());return app(pid);
}
static BOOL itemSet(id self,SEL selector,NSString *text,NSString *type){
    hold(@"prepare");
    if((flag(@"fail-string")&&[type isEqualToString:NSPasteboardTypeString])||(flag(@"fail-html")&&[type isEqualToString:NSPasteboardTypeHTML])){
        snippetRecord(@"snippet-item-failed",@{@"type":type});return NO;
    }
    return ((BOOL(*)(id,SEL,id,id))snippetOriginalSet)(self,selector,text,type);
}
static NSInteger clearBoard(id self,SEL selector){
    if([[self name] isEqualToString:NSPasteboardNameGeneral])abort();
    snippetRecord(@"snippet-clear",@{});
    return ((NSInteger(*)(id,SEL))snippetOriginalClear)(self,selector);
}
static BOOL writeBoard(id self,SEL selector,NSArray *objects){
    if([[self name] isEqualToString:NSPasteboardNameGeneral])abort();
    hold(@"write");
    lastSnippetBoard=self;
    if(flag(@"fail-write")){snippetRecord(@"snippet-write-failed",@{});return NO;}
    BOOL ok=((BOOL(*)(id,SEL,id))snippetOriginalWrite)(self,selector,objects);
    snippetRecord(@"snippet-write",@{@"ok":@(ok),@"text":[self stringForType:NSPasteboardTypeString]?:@"",@"html":[self stringForType:NSPasteboardTypeHTML]?:@"",@"changeCount":@([self changeCount]),@"items":@([[self pasteboardItems] count])});
    return ok;
}
__attribute__((visibility("default"))) uint8_t ruycast_snippet_fixture_trusted(void){
    if(!snippetInstalled)abort();snippetRecord(@"snippet-trust",@{});return !flag(@"deny-ax");
}
__attribute__((visibility("default"))) void *ruycast_snippet_fixture_create(const void *source,uint16_t key,bool down){
    (void)source;if(!snippetInstalled)abort();
    if(flag(@"fail-event"))return NULL;
    return (void *)CFBridgingRetain([@{@"key":@(key),@"down":@(down),@"flags":@0,@"tag":@0} mutableCopy]);
}
__attribute__((visibility("default"))) void ruycast_snippet_fixture_flags(void *event,uint64_t flags){
    ((__bridge NSMutableDictionary *)event)[@"flags"]=@(flags);
}
__attribute__((visibility("default"))) void ruycast_snippet_fixture_integer(void *event,uint32_t field,int64_t value){
    if(field!=42)abort();((__bridge NSMutableDictionary *)event)[@"tag"]=@(value);
}
__attribute__((visibility("default"))) void ruycast_snippet_fixture_post(uint32_t tap,void *event){
    if(!snippetInstalled||tap!=0)abort();
    NSMutableDictionary *fields=[(__bridge NSDictionary *)event mutableCopy];fields[@"pid"]=@(frontPID());
    snippetRecord(@"snippet-post",fields);
}
__attribute__((visibility("default"))) uint64_t ruycast_snippet_fixture_ready(void){return snippetInstalled;}
__attribute__((visibility("default"))) void ruycast_snippet_fixture_control(uint32_t operation){
    if(!snippetInstalled||![NSThread isMainThread])abort();
    if(operation==1){[lastSnippetBoard clearContents];[lastSnippetBoard setString:@"External fixture clipboard" forType:NSPasteboardTypeString];snippetRecord(@"snippet-external-copy",@{});}
    else if(operation==2){setFrontPID(4261);snippetRecord(@"snippet-external-focus",@{});}
    else if(operation==3){[@"new" writeToFile:[fixtureHome stringByAppendingPathComponent:@"target-new-generation"] atomically:YES encoding:NSUTF8StringEncoding error:NULL];snippetRecord(@"snippet-external-relaunch",@{});}
    else abort();
}
__attribute__((constructor)) static void installSnippet(void){
    @autoreleasepool {
        if(!fixtureHome||!installed||!getenv("RUYCAST_NATIVE_TEST_SNIPPET"))return;
        BOOL good=hook([NSWorkspace class],@selector(frontmostApplication),(IMP)frontmost);
        good=hook(object_getClass([NSRunningApplication class]),@selector(runningApplicationWithProcessIdentifier:),(IMP)lookup)&&good;
        Method set=class_getInstanceMethod([NSPasteboardItem class],@selector(setString:forType:));
        Method write=class_getInstanceMethod([NSPasteboard class],@selector(writeObjects:));
        Method clear=class_getInstanceMethod([NSPasteboard class],@selector(clearContents));
        if(!set||!write||!clear)good=NO;
        if(good){
            snippetOriginalSet=method_setImplementation(set,(IMP)itemSet);
            snippetOriginalWrite=method_setImplementation(write,(IMP)writeBoard);
            snippetOriginalClear=method_setImplementation(clear,(IMP)clearBoard);
            snippetInstalled=0x5255504153544531;
            snippetRecord(@"snippet-installed",@{});
        }
    }
}
