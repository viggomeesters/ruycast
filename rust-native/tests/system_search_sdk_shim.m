// Owned, test-only native SDK substitute. Never calls original app discovery,
// AX messaging, AppleEvent send, Automation consent or activation methods.
#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <objc/runtime.h>
#import <Carbon/Carbon.h>
#include <libproc.h>
#include <dlfcn.h>
#include <unistd.h>
static AXUIElementRef nodes[9];static NSString *mode=@"";static int selected=1;static int effects,reads,consents;static BOOL installed;
static CGPoint placement={40,50};static CGSize dimensions={600,400};
static CFTypeRef value(AXUIElementRef e,NSString *key){
    int n=0;for(int i=1;i<9;i++)if(CFEqual(e,nodes[i])){n=i;break;}if(!n)abort();reads++;
    NSString *title=@[@"",@"Owned App",@"Owned Window",@"Owned Minimized",@"",@"File",@"",@"Open Owned…",@"Disabled Owned"][n];
    if(n==2 && [mode isEqualToString:@"renamed"])title=@"Changed";
    NSString *role=@[@"",@"AXApplication",@"AXWindow",@"AXWindow",@"AXMenuBar",@"AXMenuBarItem",@"AXMenu",@"AXMenuItem",@"AXMenuItem"][n];
    if([key isEqualToString:@"AXRole"])return (__bridge CFTypeRef)role;
    if([key isEqualToString:@"AXTitle"])return (__bridge CFTypeRef)title;
    if([key isEqualToString:@"AXEnabled"])return n==8?kCFBooleanFalse:kCFBooleanTrue;
    if([key isEqualToString:@"AXMinimized"] && (n==2 || n==3))return n==3 && ![mode isEqualToString:@"restored"]?kCFBooleanTrue:kCFBooleanFalse;
    if([key isEqualToString:@"AXWindows"] && n==1)return (__bridge CFTypeRef)@[(__bridge id)nodes[2],(__bridge id)nodes[3]];
    if([key isEqualToString:@"AXMenuBar"] && n==1)return nodes[4];
    if([key isEqualToString:@"AXFocusedWindow"] && n==1)return nodes[2];
    if([key isEqualToString:@"AXChildren"]){if(n==4)return (__bridge CFTypeRef)@[(__bridge id)nodes[5]];if(n==5)return (__bridge CFTypeRef)@[(__bridge id)nodes[6]];if(n==6)return (__bridge CFTypeRef)@[(__bridge id)nodes[7],(__bridge id)nodes[8]];return (__bridge CFTypeRef)@[];}
    if([key isEqualToString:@"AXMenuItemCmdChar"] && n==7)return (__bridge CFTypeRef)@"o";
    if([key isEqualToString:@"AXMenuItemCmdModifiers"] && n==7)return (__bridge CFTypeRef)@0;
    return NULL;
}
static Boolean trust(void){return ![mode isEqualToString:@"denied"];}
static AXUIElementRef app(pid_t p){if(p!=730001)abort();return (AXUIElementRef)CFRetain(nodes[1]);}
static AXError pid(AXUIElementRef e,pid_t *p){if(!e)abort();*p=730001;return 0;}
static AXError timeout(AXUIElementRef e,float seconds){if(!e || !(seconds>0 && seconds<=0.121))abort();return 0;}
static AXError attr(AXUIElementRef e,CFStringRef k,CFTypeRef *out){
    if(CFEqual(e,nodes[2])&&(CFEqual(k,CFSTR("AXPosition"))||CFEqual(k,CFSTR("AXSize")))){
        reads++;if([mode isEqualToString:@"frame-denied"]){*out=NULL;return kAXErrorAttributeUnsupported;}
        *out=CFEqual(k,CFSTR("AXPosition"))?AXValueCreate(kAXValueCGPointType,&placement):AXValueCreate(kAXValueCGSizeType,&dimensions);return 0;
    }
    CFTypeRef v=value(e,(__bridge NSString *)k);*out=v?CFRetain(v):NULL;return v?0:kAXErrorAttributeUnsupported;}
static AXError count(AXUIElementRef e,CFStringRef k,CFIndex *out){CFTypeRef v=value(e,(__bridge NSString *)k);if(!v||CFGetTypeID(v)!=CFArrayGetTypeID())return kAXErrorAttributeUnsupported;*out=CFArrayGetCount(v);return 0;}
static AXError values(AXUIElementRef e,CFStringRef k,CFIndex start,CFIndex max,CFArrayRef *out){CFTypeRef v=value(e,(__bridge NSString *)k);if(!v||CFGetTypeID(v)!=CFArrayGetTypeID())return kAXErrorAttributeUnsupported;NSArray *a=(__bridge NSArray*)v;NSUInteger n=MIN((NSUInteger)max,a.count-(NSUInteger)start);*out=(CFArrayRef)CFBridgingRetain([a subarrayWithRange:NSMakeRange(start,n)]);return 0;}
static AXError set(AXUIElementRef e,CFStringRef k,CFTypeRef v){
    if(CFEqual(e,nodes[2])&&CFEqual(k,CFSTR("AXPosition"))){if(AXValueGetType(v)!=kAXValueCGPointType||!AXValueGetValue(v,kAXValueCGPointType,&placement))abort();effects++;return [mode isEqualToString:@"frame-cannot-complete"]?kAXErrorCannotComplete:0;}
    if(CFEqual(e,nodes[2])&&CFEqual(k,CFSTR("AXSize"))){if(AXValueGetType(v)!=kAXValueCGSizeType||!AXValueGetValue(v,kAXValueCGSizeType,&dimensions))abort();if([mode isEqualToString:@"frame-constrained"])dimensions.width=100;effects++;return [mode isEqualToString:@"frame-cannot-complete"]?kAXErrorCannotComplete:0;}
    if(!CFEqual(e,nodes[3])||!CFEqual(k,CFSTR("AXMinimized"))||!CFEqual(v,kCFBooleanFalse))abort();effects++;mode=@"restored";return 0;}
static AXError action(AXUIElementRef e,CFStringRef k){if(![(__bridge NSString*)k isEqualToString:@"AXRaise"] && ![(__bridge NSString*)k isEqualToString:@"AXPress"])abort();if(!e)abort();effects++;return 0;}
__attribute__((visibility("default"))) int ruycast_system_fixture_proc(pid_t p,int flavor,uint64_t arg,void *out,int size){if(p!=730001||flavor!=PROC_PIDTBSDINFO||arg!=0||size!=sizeof(struct proc_bsdinfo))return 0;struct proc_bsdinfo info={0};info.pbi_pid=p;info.pbi_uid=getuid();info.pbi_start_tvsec=[mode isEqualToString:@"restarted"]?12:11;info.pbi_start_tvusec=22;memcpy(out,&info,sizeof(info));return sizeof(info);}
static OSStatus consent(const AEAddressDesc *target,AEEventClass c,AEEventID i,Boolean ask){if(!target||c!='****'||i!='****'||ask!=1)abort();consents++;return noErr;}
#define INTERPOSE(replacement,replacee) __attribute__((used)) static struct {const void *a;const void *b;} interpose_##replacee __attribute__((section("__DATA,__interpose")))={(const void*)(replacement),(const void*)(replacee)};
INTERPOSE(trust,AXIsProcessTrusted) INTERPOSE(app,AXUIElementCreateApplication) INTERPOSE(pid,AXUIElementGetPid) INTERPOSE(timeout,AXUIElementSetMessagingTimeout) INTERPOSE(attr,AXUIElementCopyAttributeValue) INTERPOSE(count,AXUIElementGetAttributeValueCount) INTERPOSE(values,AXUIElementCopyAttributeValues) INTERPOSE(set,AXUIElementSetAttributeValue) INTERPOSE(action,AXUIElementPerformAction) INTERPOSE(consent,AEDeterminePermissionToAutomateTarget)
@interface OwnedSystemApp:NSObject
@end
@implementation OwnedSystemApp
-(pid_t)processIdentifier{return 730001;}
-(BOOL)isTerminated{return NO;}
-(NSString*)localizedName{return @"Owned Native App";}
-(NSString*)bundleIdentifier{return @"com.apple.Safari";}
-(NSURL*)bundleURL{return [NSURL fileURLWithPath:@"/Applications/Safari.app"];}
-(NSApplicationActivationPolicy)activationPolicy{return NSApplicationActivationPolicyRegular;}
-(BOOL)activateWithOptions:(NSApplicationActivationOptions)options{if(![NSThread isMainThread]||options!=0)abort();effects++;if([mode isEqualToString:@"mutate-activate"])mode=@"renamed";return YES;}
@end
static OwnedSystemApp *owned;
static NSObject *ownedWorkspace;
static id shared(id self,SEL cmd){(void)self;(void)cmd;return ownedWorkspace;}
static id running(id self,SEL cmd){(void)self;(void)cmd;reads++;return @[owned];}
static id front(id self,SEL cmd){(void)self;(void)cmd;reads++;return owned;}
static id runningBundle(id self,SEL cmd,NSString *bundle){(void)self;(void)cmd;reads++;if(![bundle isEqualToString:@"com.apple.Safari"])abort();return @[owned];}
static id runningPid(id self,SEL cmd,pid_t p){(void)self;(void)cmd;if(p!=730001)abort();return owned;}
static NSAppleEventDescriptor *list(NSArray *items){NSAppleEventDescriptor *d=[NSAppleEventDescriptor listDescriptor];for(NSAppleEventDescriptor *v in items)[d insertDescriptor:v atIndex:d.numberOfItems+1];return d;}
static NSAppleEventDescriptor *event(id self,SEL cmd,NSAppleEventSendOptions flags,NSTimeInterval seconds,NSError **error){
    (void)error;
    (void)cmd;if(flags!=(3|16|4096|0x20000)||seconds<=0||seconds>8)abort();NSAppleEventDescriptor *e=self;reads++;
    NSAppleEventDescriptor *reply=[NSAppleEventDescriptor appleEventWithEventClass:'aevt' eventID:'ansr' targetDescriptor:nil returnID:0 transactionID:0];
    if([mode isEqualToString:@"automation-denied"]){[reply setParamDescriptor:[NSAppleEventDescriptor descriptorWithInt32:-1743] forKeyword:'errn'];return reply;}
    if(e.eventClass!='core'||[e attributeDescriptorForKeyword:keyAddressAttr].descriptorType!=typeKernelProcessID)abort();
    NSAppleEventDescriptor *ref=[e paramDescriptorForKeyword:'----'];OSType key=[ref descriptorForKeyword:'seld'].typeCodeValue;BOOL current=[[[ref descriptorForKeyword:'from'] descriptorForKeyword:'seld'] typeCodeValue]=='cTab';NSAppleEventDescriptor *v=nil;
    if(e.eventID=='setd'){
        NSAppleEventDescriptor *data=[e paramDescriptorForKeyword:'data'];if(key=='cTab'){if([[data descriptorForKeyword:'want'] typeCodeValue]!='bTab')abort();selected=[[data descriptorForKeyword:'seld'] int32Value];effects++;if([mode isEqualToString:@"mutate-selection"])mode=@"tab-changed";}else if(key=='pidx'){effects++;}else abort();
    }else if(e.eventID=='getd'){
        if(key=='ID  ')v=list(@[[NSAppleEventDescriptor descriptorWithInt32:51]]);
        else if(current){if(key=='pidx')v=[NSAppleEventDescriptor descriptorWithInt32:selected];else if(key=='pnam')v=[NSAppleEventDescriptor descriptorWithString:[mode isEqualToString:@"tab-changed"]?@"Changed":@"Owned Native Tab"];else if(key=='pURL')v=[NSAppleEventDescriptor descriptorWithString:@"https://example.invalid/native"];else abort();}
        else if(key=='pnam')v=list(@[[NSAppleEventDescriptor descriptorWithString:[mode isEqualToString:@"tab-changed"]?@"Changed":@"Owned Native Tab"]]);
        else if(key=='pURL')v=list(@[[NSAppleEventDescriptor descriptorWithString:@"https://example.invalid/native"]]);else abort();
        [reply setParamDescriptor:v forKeyword:'----'];
    }else abort();return reply;
}
__attribute__((visibility("default"))) uint64_t ruycast_system_fixture_ready(void){return installed?0x525553595354454d:0;}
__attribute__((visibility("default"))) void ruycast_system_fixture_mode(const char *v){mode=[NSString stringWithUTF8String:v];selected=1;effects=0;}
__attribute__((visibility("default"))) int ruycast_system_fixture_effects(void){return effects;}
__attribute__((visibility("default"))) int ruycast_system_fixture_reads(void){return reads;}
__attribute__((visibility("default"))) int ruycast_system_fixture_consents(void){return consents;}
__attribute__((constructor))static void install(void){@autoreleasepool{
    const char *root=getenv("RUYCAST_SYSTEM_SDK_FIXTURE_HOME");if(!root)return;NSString *home=[NSString stringWithUTF8String:root];NSString *marker=[NSString stringWithContentsOfFile:[home stringByAppendingPathComponent:@".ruycast-test-fixture"] encoding:NSUTF8StringEncoding error:NULL];if(![marker isEqualToString:@"own-disposable-fixture-v1"])abort();
    // These create local opaque AX handles for nonexistent fixture PIDs. No
    // message is sent; every subsequent AX operation above is substituted.
    
    for(int i=1;i<9;i++)nodes[i]=AXUIElementCreateApplication(730001+i);
    
    owned=[OwnedSystemApp new];ownedWorkspace=[NSObject new];
    method_setImplementation(class_getClassMethod([NSWorkspace class],@selector(sharedWorkspace)),(IMP)shared);
    class_addMethod([NSObject class],@selector(runningApplications),(IMP)running,"@@:");
    class_addMethod([NSObject class],@selector(frontmostApplication),(IMP)front,"@@:");
    method_setImplementation(class_getInstanceMethod([NSWorkspace class],@selector(runningApplications)),(IMP)running);
    method_setImplementation(class_getInstanceMethod([NSWorkspace class],@selector(frontmostApplication)),(IMP)front);
    method_setImplementation(class_getClassMethod([NSRunningApplication class],@selector(runningApplicationsWithBundleIdentifier:)),(IMP)runningBundle);
    method_setImplementation(class_getClassMethod([NSRunningApplication class],@selector(runningApplicationWithProcessIdentifier:)),(IMP)runningPid);
    method_setImplementation(class_getInstanceMethod([NSAppleEventDescriptor class],@selector(sendEventWithOptions:timeout:error:)),(IMP)event);
    installed=YES;
}}
