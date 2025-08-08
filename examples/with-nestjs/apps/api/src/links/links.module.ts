import { Module } from '@nestjs/common';

import { LinksService } from './links.service';
import { LinksController } from './links.controller';

@Module({
  controllers: [LinksController],
  providers: [LinksService],
})
export class LinksModule {}
