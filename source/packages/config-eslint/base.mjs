import js from "@eslint/js";
import tseslint from "typescript-eslint";

export default tseslint.config(
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    rules: {
      /**
       * 이 저장소는 "쓰지 않는다"를 `_` 접두사로 표시한다. 기본값은 그 관용구를
       * 모르므로 의도적으로 버린 값까지 오류로 잡는다.
       *
       * 특히 `ignoreRestSiblings` 는 기본이 false 인데, 키 하나를 빼는 관용구
       * (`const { submittedAt: _drop, ...rest } = obj`)가 여기에 걸린다. 이건
       * 지울 수 없는 코드다 — 지우면 그 키가 도로 들어간다.
       */
      "@typescript-eslint/no-unused-vars": [
        "error",
        {
          argsIgnorePattern: "^_",
          varsIgnorePattern: "^_",
          caughtErrorsIgnorePattern: "^_",
          destructuredArrayIgnorePattern: "^_",
          ignoreRestSiblings: true,
        },
      ],
    },
  },
  { ignores: ["dist/**", ".next/**", ".turbo/**", "node_modules/**"] }
);
