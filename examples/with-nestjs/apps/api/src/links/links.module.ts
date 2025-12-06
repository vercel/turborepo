import { Module } from '@nestjs/common';
import { LinksController } from './links.controller';
import { LinksService } from './links.service';

@Module({
	controllers: [LinksController],
	providers: [LinksService],
})
export class LinksModule {}
