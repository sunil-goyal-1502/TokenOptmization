export declare function nativeAvailable(): boolean;
export declare function compileJson(
  messagesJson: string,
  optionsJson?: string,
): Promise<string>;
export declare function compareJson(
  messagesJson: string,
  optionsJson?: string,
): Promise<string>;
export declare function analyzeJson(messagesJson: string): string;
