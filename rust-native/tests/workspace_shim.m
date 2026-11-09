// Test-only NSWorkspace method boundary in this own disposable process.
// Every intercepted operation returns synthetic data; no original open or
// application-resolution implementation is ever called.
#import <AppKit/AppKit.h>
#import <objc/runtime.h>
#import <dispatch/dispatch.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/stat.h>
#include <unistd.h>
static IMP originalClipboardRead;
static uint64_t installed;
static NSString *fixtureHome;
static NSMutableArray *held;
static NSLock *heldLock;
static pthread_mutex_t traceLock=PTHREAD_MUTEX_INITIALIZER;
typedef void (^Completion)(NSRunningApplication *,NSError *);
static void record(NSString *phase,NSDictionary *fields) {
    @autoreleasepool {
        NSMutableDictionary *entry=[fields mutableCopy];entry[@"phase"]=phase;entry[@"mainThread"]=@([NSThread isMainThread]);
        NSData *data=[NSJSONSerialization dataWithJSONObject:entry options:0 error:NULL];
        pthread_mutex_lock(&traceLock);
        FILE *f=fopen([[fixtureHome stringByAppendingPathComponent:@"workspace-trace.jsonl"] fileSystemRepresentation],"a");
        if(f){fwrite(data.bytes,1,data.length,f);fwrite("\n",1,1,f);fclose(f);}
        pthread_mutex_unlock(&traceLock);
    }
}
static NSString *lastHideTrace;
static void requireHide(void){
    if(!getenv("RUYCAST_NATIVE_TEST_REQUIRE_HIDE"))return;
    NSString *text=[NSString stringWithContentsOfFile:[fixtureHome stringByAppendingPathComponent:@"p/native-hide.jsonl"] encoding:NSUTF8StringEncoding error:NULL];
    NSString *line=[[text stringByTrimmingCharactersInSet:[NSCharacterSet newlineCharacterSet]] componentsSeparatedByString:@"\n"].lastObject;
    NSDictionary *ack=line?[NSJSONSerialization JSONObjectWithData:[line dataUsingEncoding:NSUTF8StringEncoding] options:0 error:NULL]:nil;
    if(!ack||![ack[@"mainThread"] boolValue]||[ack[@"afterVisible"] boolValue]||[lastHideTrace isEqualToString:ack[@"trace"]]){record(@"missing-exact-hide",@{});abort();}
    lastHideTrace=ack[@"trace"];
}
static void entered(void){[[@"entered" dataUsingEncoding:NSUTF8StringEncoding] writeToFile:[fixtureHome stringByAppendingPathComponent:@"open-started"] atomically:YES];}
static NSString *modeForURL(NSURL *url){
    NSString *value=url.isFileURL?url.lastPathComponent:url.host;
    if([value hasPrefix:@"Fake-"])value=[[value substringFromIndex:5] stringByDeletingPathExtension];
    return value?:@"success";
}
static NSError *fakeError(NSString *mode){
    NSString *message=[mode isEqualToString:@"large-error"]?[@"Fixture Ω😀 " stringByPaddingToLength:100000 withString:@"Ω😀" startingAtIndex:0]:@"Fixture native error.";
    return [NSError errorWithDomain:@"dev.ruycast.fixture" code:7 userInfo:@{NSLocalizedDescriptionKey:message}];
}
static NSURL *resolve(id self,SEL cmd,NSString *identifier){
    (void)self;(void)cmd;record(@"resolve",@{@"bundleID":identifier?:@""});
    if(![identifier hasPrefix:@"dev.fixture."]||[identifier isEqualToString:@"dev.fixture.missing"])return nil;
    NSString *mode=[identifier substringFromIndex:@"dev.fixture.".length];
    if([mode hasPrefix:@"hold-resolve"]){
        [[@"entered" dataUsingEncoding:NSUTF8StringEncoding] writeToFile:[fixtureHome stringByAppendingPathComponent:@"resolve-entered"] atomically:YES];
        while(![[NSFileManager defaultManager] fileExistsAtPath:[fixtureHome stringByAppendingPathComponent:@"workspace-release"]])usleep(2000);
    }
    if([mode isEqualToString:@"slow-resolve"])usleep(150000);
    if([mode isEqualToString:@"invalid-location"])return [NSURL URLWithString:@"https://fixture.invalid/App.app"];
    return [NSURL fileURLWithPath:[fixtureHome stringByAppendingPathComponent:[NSString stringWithFormat:@"Fake-%@.app",mode]]];
}
static BOOL openDefault(id self,SEL cmd,NSURL *url){
    (void)self;(void)cmd;requireHide();record(@"open-default",@{@"url":url.absoluteString?:@""});entered();
    NSString *mode=modeForURL(url);if([mode isEqualToString:@"slow-sync"])usleep(150000);
    if([mode isEqualToString:@"hold-sync"]){while(![[NSFileManager defaultManager] fileExistsAtPath:[fixtureHome stringByAppendingPathComponent:@"workspace-release"]])usleep(2000);}
    return ![mode isEqualToString:@"reject"];
}
static void complete(Completion callback,NSString *mode){
    record(@"callback",@{@"mode":mode});
    if([mode isEqualToString:@"error"]||[mode isEqualToString:@"large-error"]){callback(nil,fakeError(mode));}
    else if([mode isEqualToString:@"nil-result"]){callback(nil,nil);}
    else {callback([NSRunningApplication currentApplication],nil);}
}
static void opening(NSURL *url,Completion callback,NSString *selector){
    requireHide();NSString *mode=modeForURL(url);record(selector,@{@"url":url.absoluteString?:@"",@"mode":mode});entered();
    if(!callback)return;
    if([mode isEqualToString:@"hold"]){[heldLock lock];[held addObject:[callback copy]];[heldLock unlock];return;}
    if([mode isEqualToString:@"duplicate"]){complete(callback,@"success");complete(callback,@"error");return;}
    if([mode isEqualToString:@"inline"]){complete(callback,@"success");return;}
    int64_t delay=[mode isEqualToString:@"slow"]?200000000:1000000;
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW,delay),dispatch_get_global_queue(QOS_CLASS_DEFAULT,0),^{@autoreleasepool{complete(callback,mode);}});
}
static void openApp(id self,SEL cmd,NSURL *url,NSWorkspaceOpenConfiguration *configuration,Completion callback){
    (void)self;(void)cmd;(void)configuration;opening(url,callback,@"open-app");
}
static void openWith(id self,SEL cmd,NSArray<NSURL *> *urls,NSURL *application,NSWorkspaceOpenConfiguration *configuration,Completion callback){
    (void)self;(void)cmd;(void)configuration;record(@"open-with-targets",@{@"urls":[urls valueForKey:@"absoluteString"]?:@[]});opening(application,callback,@"open-with");
}
__attribute__((visibility("default"))) uint64_t ruycast_workspace_fixture_ready(void){return installed;}
__attribute__((visibility("default"))) void ruycast_workspace_fixture_release(void){
    @autoreleasepool {
        [heldLock lock];NSArray *callbacks=[held copy];[held removeAllObjects];[heldLock unlock];
        for(Completion callback in callbacks)complete(callback,@"success");
    }
}
static BOOL hook(Class cls,SEL selector,IMP implementation){Method method=class_getInstanceMethod(cls,selector);if(!method)return NO;method_setImplementation(method,implementation);return YES;}
static id denyGeneral(id self,SEL selector) {
    (void)self;(void)selector;record(@"unexpected-general-pasteboard",@{});abort();
}
static id clipboardRead(id self,SEL selector,id type) {
    if([[self name] isEqualToString:@"NSGeneralPboard"]){record(@"unexpected-general-read",@{});abort();}
    record(@"clipboard-read",@{});
    if(getenv("RUYCAST_NATIVE_TEST_CLIPBOARD_TEXT") && strcmp(getenv("RUYCAST_NATIVE_TEST_CLIPBOARD_TEXT"),"hold-main")==0){
        [[@"entered" dataUsingEncoding:NSUTF8StringEncoding] writeToFile:[fixtureHome stringByAppendingPathComponent:@"clipboard-entered"] atomically:YES];
        while(![[NSFileManager defaultManager] fileExistsAtPath:[fixtureHome stringByAppendingPathComponent:@"workspace-release"]])usleep(2000);
    }
    return ((id(*)(id,SEL,id))originalClipboardRead)(self,selector,type);
}
__attribute__((constructor)) static void install(void){
    @autoreleasepool {
        const char *home=getenv("RUYCAST_WORKSPACE_FIXTURE_HOME");if(!home)return;
        fixtureHome=[[NSString alloc] initWithUTF8String:home];
        NSString *marker=[fixtureHome stringByAppendingPathComponent:@".ruycast-test-fixture"];
        NSData *value=[NSData dataWithContentsOfFile:marker];if(![value isEqualToData:[@"own-disposable-fixture-v1" dataUsingEncoding:NSUTF8StringEncoding]])return;
        held=[NSMutableArray new];heldLock=[NSLock new];Class cls=[NSWorkspace class];
        if(getenv("RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD")){
        Method read=class_getInstanceMethod([NSPasteboard class],@selector(stringForType:));
        originalClipboardRead=method_getImplementation(read);
        hook([NSPasteboard class],@selector(stringForType:),(IMP)clipboardRead);
        if(strcmp(getenv("RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD"),"trace")!=0)hook(object_getClass([NSPasteboard class]),@selector(generalPasteboard),(IMP)denyGeneral);
        }
        BOOL good=hook(cls,@selector(URLForApplicationWithBundleIdentifier:),(IMP)resolve);
        good=hook(cls,@selector(openURL:),(IMP)openDefault)&&good;
        good=hook(cls,@selector(openApplicationAtURL:configuration:completionHandler:),(IMP)openApp)&&good;
        good=hook(cls,@selector(openURLs:withApplicationAtURL:configuration:completionHandler:),(IMP)openWith)&&good;
        if(good){installed=0x52555943;record(@"installed",@{@"selectors":@4});}
    }
}
