CREATE TABLE "User" (
	"id" TEXT NOT NULL,
	"email" TEXT NOT NULL,
	CONSTRAINT "User_pkey" PRIMARY KEY ("id")
);

CREATE UNIQUE INDEX "User_email_key" ON "User"("email");
